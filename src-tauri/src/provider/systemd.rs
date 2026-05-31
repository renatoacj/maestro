//! Provider para `systemd --user` via session D-Bus (sem root, sem shell).
//!
//! Fala diretamente com `org.freedesktop.systemd1.Manager` no barramento de sessão.
//! Nenhum comando é interpolado em shell: todas as ações são chamadas D-Bus tipadas.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use async_trait::async_trait;
use zbus::zvariant::OwnedObjectPath;
use zbus::Connection;

use crate::error::{Error, Result};
use crate::model::{Action, Health, Job, JobDetail, JobKind, JobState, Resources, Schedule};
use crate::provider::JobProvider;

pub const PROVIDER_ID: &str = "systemd-user";

const DEST: &str = "org.freedesktop.systemd1";
const UNIT_IFACE: &str = "org.freedesktop.systemd1.Unit";
const SERVICE_IFACE: &str = "org.freedesktop.systemd1.Service";
const TIMER_IFACE: &str = "org.freedesktop.systemd1.Timer";

/// Entrada de `ExecStart`: (caminho, argv, ignore_failure, …timestamps…, pid, code, status).
type ExecEntry = (String, Vec<String>, bool, u64, u64, u64, u64, u32, i32, i32);

/// Mudança retornada por Enable/DisableUnitFiles: (tipo, nome, destino).
type UnitFileChange = (String, String, String);

/// Tupla retornada por `ListUnits`, na ordem definida pela API do systemd.
type UnitInfo = (
    String,          // 0 nome (ex: "my-worker.service")
    String,          // 1 descrição
    String,          // 2 load state
    String,          // 3 active state ("active", "failed", …)
    String,          // 4 sub state
    String,          // 5 followed
    OwnedObjectPath, // 6 object path da unit
    u32,             // 7 job id
    String,          // 8 job type
    OwnedObjectPath, // 9 job object path
);

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait Manager {
    fn list_units(&self) -> zbus::Result<Vec<UnitInfo>>;
    fn list_unit_files(&self) -> zbus::Result<Vec<(String, String)>>;
    fn get_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn restart_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn enable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
        force: bool,
    ) -> zbus::Result<(bool, Vec<UnitFileChange>)>;
    fn disable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
    ) -> zbus::Result<Vec<UnitFileChange>>;

    /// Habilita a emissão de sinais (UnitNew/JobRemoved/…) neste cliente.
    fn subscribe(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn job_removed(
        &self,
        id: u32,
        job: OwnedObjectPath,
        unit: String,
        result: String,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    fn unit_new(&self, id: String, unit: OwnedObjectPath) -> zbus::Result<()>;

    #[zbus(signal)]
    fn unit_removed(&self, id: String, unit: OwnedObjectPath) -> zbus::Result<()>;
}

/// Provider de user-services e user-timers do systemd.
pub struct SystemdUserProvider {
    conn: Connection,
    /// Última leitura de CPU por unit: (CPUUsageNSec, instante). Usada para
    /// derivar CPU% a partir do delta entre duas amostras.
    cpu_cache: Mutex<HashMap<String, (u64, Instant)>>,
}

impl SystemdUserProvider {
    /// Conecta ao session bus do usuário. Falha se não houver session D-Bus.
    pub async fn connect() -> Result<Self> {
        let conn = Connection::session().await?;
        Ok(Self {
            conn,
            cpu_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Deriva CPU% a partir do tempo de CPU acumulado (ns) entre duas amostras.
    /// A primeira amostra de uma unit retorna `None` (ainda não há delta).
    /// Pode passar de 100% (soma entre cores) — comportamento esperado, como no `top`.
    fn compute_cpu_pct(&self, name: &str, nsec_now: u64) -> Option<f32> {
        let now = Instant::now();
        let mut cache = self.cpu_cache.lock().unwrap();
        let prev = cache.insert(name.to_string(), (nsec_now, now));
        let (nsec_prev, at_prev) = prev?;
        let elapsed = now.duration_since(at_prev).as_nanos();
        if elapsed == 0 || nsec_now < nsec_prev {
            return None;
        }
        let pct = (nsec_now - nsec_prev) as f64 / elapsed as f64 * 100.0;
        Some(pct as f32)
    }

    async fn manager(&self) -> Result<ManagerProxy<'_>> {
        Ok(ManagerProxy::new(&self.conn).await?)
    }

    /// Lê recursos de um job pela interface apropriada (best-effort).
    async fn read_resources(&self, name: &str) -> Resources {
        let mut res = Resources::default();
        let Ok(mgr) = self.manager().await else {
            return res;
        };
        let Ok(path) = mgr.get_unit(name).await else {
            return res;
        };
        if let Ok(proxy) = zbus::Proxy::new(&self.conn, DEST, path, SERVICE_IFACE).await {
            // u64::MAX significa "não disponível" no systemd.
            if let Ok(mem) = proxy.get_property::<u64>("MemoryCurrent").await {
                if mem != u64::MAX {
                    res.mem_bytes = Some(mem);
                }
            }
            if let Ok(pid) = proxy.get_property::<u32>("MainPID").await {
                if pid != 0 {
                    res.pids.push(pid);
                }
            }
            // CPU acumulada em nanossegundos → CPU% via delta entre amostras.
            if let Ok(nsec) = proxy.get_property::<u64>("CPUUsageNSec").await {
                if nsec != u64::MAX {
                    res.cpu_pct = self.compute_cpu_pct(name, nsec);
                }
            }
        }
        res
    }

    /// Lê próximo/último disparo de um timer (best-effort). usec→sec desde epoch.
    async fn read_schedule(&self, timer_name: &str) -> Option<Schedule> {
        let mgr = self.manager().await.ok()?;
        let path = mgr.get_unit(timer_name).await.ok()?;
        let proxy = zbus::Proxy::new(&self.conn, DEST, path, TIMER_IFACE)
            .await
            .ok()?;
        let to_secs =
            |usec: u64| (usec != 0 && usec != u64::MAX).then_some((usec / 1_000_000) as i64);
        let next = proxy
            .get_property::<u64>("NextElapseUSecRealtime")
            .await
            .ok()
            .and_then(to_secs);
        let last = proxy
            .get_property::<u64>("LastTriggerUSec")
            .await
            .ok()
            .and_then(to_secs);
        Some(Schedule {
            next_run: next,
            last_run: last,
        })
    }
}

/// Mapeia o `active_state` do systemd para o nosso [`JobState`].
fn map_state(active_state: &str) -> JobState {
    match active_state {
        "active" => JobState::Active,
        "inactive" => JobState::Inactive,
        "failed" => JobState::Failed,
        "activating" => JobState::Activating,
        "deactivating" => JobState::Deactivating,
        _ => JobState::Unknown,
    }
}

/// Deriva saúde de alto nível a partir do estado. Mantém a leitura calma:
/// inativo não é alarme (timers e oneshots vivem inativos).
fn map_health(state: JobState) -> Health {
    match state {
        JobState::Failed => Health::Failed,
        JobState::Activating | JobState::Deactivating => Health::Degraded,
        _ => Health::Ok,
    }
}

#[async_trait]
impl JobProvider for SystemdUserProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    async fn available(&self) -> bool {
        self.manager().await.is_ok()
    }

    async fn list(&self) -> Result<Vec<Job>> {
        let mgr = self.manager().await?;
        let home = std::env::var("HOME").unwrap_or_default();

        // Unidades DO USUÁRIO = aquelas cujo arquivo vive sob o $HOME
        // (~/.config/systemd/user, ~/.local/share/systemd/user). Isso exclui o
        // plumbing do sistema (/usr/lib, /etc, /run) e os apps transitórios
        // (`app-*` lançados pelo desktop, como navegadores), que não têm fragmento
        // sob o home do usuário. Resultado: só o que o usuário de fato criou.
        let mut user_files: HashMap<String, String> = HashMap::new(); // nome -> estado do arquivo
        for (path, state) in mgr.list_unit_files().await? {
            if home.is_empty() || !path.starts_with(&home) {
                continue;
            }
            let Some(name) = path.rsplit('/').next() else {
                continue;
            };
            if name.ends_with(".service") || name.ends_with(".timer") {
                user_files.insert(name.to_string(), state);
            }
        }

        // Estado atual das unidades carregadas, indexado por nome.
        let loaded: HashMap<String, UnitInfo> = mgr
            .list_units()
            .await?
            .into_iter()
            .map(|u| (u.0.clone(), u))
            .collect();

        let mut jobs = Vec::new();
        for (name, file_state) in &user_files {
            let kind = if name.ends_with(".timer") {
                JobKind::Scheduled
            } else {
                JobKind::Service
            };

            // Carregada → usa estado/descrição reais; só no disco → Inactive.
            let (description, state) = match loaded.get(name) {
                Some(u) => (u.1.clone(), map_state(&u.3)),
                None => (String::new(), JobState::Inactive),
            };

            let enabled = file_state == "enabled" || file_state == "enabled-runtime";

            let schedule = if kind == JobKind::Scheduled {
                self.read_schedule(name).await
            } else {
                None
            };

            jobs.push(Job {
                id: Job::global_id(PROVIDER_ID, name),
                provider: PROVIDER_ID.to_string(),
                local_id: name.clone(),
                kind,
                name: name.clone(),
                description,
                command: None, // preenchido sob demanda (ExecStart) em incremento futuro
                state,
                enabled,
                schedule,
                resources: Resources::default(), // recursos vêm via metrics(), sob demanda
                health: map_health(state),
            });
        }

        jobs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(jobs)
    }

    async fn control(&self, local_id: &str, action: Action) -> Result<()> {
        let mgr = self.manager().await?;
        match action {
            Action::Start => {
                mgr.start_unit(local_id, "replace").await?;
            }
            Action::Stop => {
                mgr.stop_unit(local_id, "replace").await?;
            }
            Action::Restart => {
                mgr.restart_unit(local_id, "replace").await?;
            }
            Action::Enable => {
                mgr.enable_unit_files(&[local_id], false, false).await?;
            }
            Action::Disable => {
                mgr.disable_unit_files(&[local_id], false).await?;
            }
            Action::TriggerNow => {
                // Para um timer, dispara o .service correspondente; para um service, é Start.
                let target = if let Some(stem) = local_id.strip_suffix(".timer") {
                    format!("{stem}.service")
                } else {
                    local_id.to_string()
                };
                mgr.start_unit(&target, "replace").await?;
            }
        }
        Ok(())
    }

    async fn metrics(&self, local_id: &str) -> Result<Resources> {
        if !self.available().await {
            return Err(Error::Unavailable(PROVIDER_ID.into()));
        }
        Ok(self.read_resources(local_id).await)
    }

    async fn detail(&self, local_id: &str) -> Result<JobDetail> {
        let mgr = self.manager().await?;
        let path = mgr.get_unit(local_id).await?;
        let mut d = JobDetail::default();

        // Interface Unit: caminho do arquivo e desde quando está ativo.
        if let Ok(unit) = zbus::Proxy::new(&self.conn, DEST, path.clone(), UNIT_IFACE).await {
            if let Ok(fp) = unit.get_property::<String>("FragmentPath").await {
                if !fp.is_empty() {
                    d.fragment_path = Some(fp);
                }
            }
            if let Ok(ts) = unit.get_property::<u64>("ActiveEnterTimestamp").await {
                if ts != 0 && ts != u64::MAX {
                    d.since = Some((ts / 1_000_000) as i64);
                }
            }
        }

        // Interface Service: comando, motivo de término e código de saída.
        if local_id.ends_with(".service") {
            if let Ok(svc) = zbus::Proxy::new(&self.conn, DEST, path, SERVICE_IFACE).await {
                if let Ok(execs) = svc.get_property::<Vec<ExecEntry>>("ExecStart").await {
                    if let Some(first) = execs.first() {
                        d.command = Some(first.1.join(" "));
                    }
                }
                if let Ok(res) = svc.get_property::<String>("Result").await {
                    if !res.is_empty() {
                        d.exit_reason = Some(res);
                    }
                }
                if let Ok(code) = svc.get_property::<i32>("ExecMainStatus").await {
                    d.exit_code = Some(code);
                }
            }
        }
        Ok(d)
    }

    async fn logs(&self, local_id: &str, lines: u32) -> Result<Vec<String>> {
        // journalctl chamado com argv fixo — nenhuma interpolação de shell.
        let n = lines.clamp(1, 5000).to_string();
        let out = tokio::process::Command::new("journalctl")
            .args([
                "--user",
                "-u",
                local_id,
                "-n",
                &n,
                "--no-pager",
                "-o",
                "short-iso",
            ])
            .output()
            .await
            .map_err(|e| Error::Other(format!("journalctl falhou: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(Error::Other(format!("journalctl: {}", err.trim())));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect())
    }

    async fn watch(&self) -> Option<crate::provider::ChangeStream> {
        use futures_util::StreamExt;

        let conn = self.conn.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<()>(16);

        // Task dedicada possui o proxy e os streams de sinal (mantém tudo 'static).
        tokio::spawn(async move {
            let Ok(mgr) = ManagerProxy::new(&conn).await else {
                return;
            };
            if mgr.subscribe().await.is_err() {
                return;
            }
            let (mut jr, mut un, mut ur) = match tokio::try_join!(
                mgr.receive_job_removed(),
                mgr.receive_unit_new(),
                mgr.receive_unit_removed(),
            ) {
                Ok(streams) => streams,
                Err(_) => return,
            };

            loop {
                let changed = tokio::select! {
                    s = jr.next() => s.is_some(),
                    s = un.next() => s.is_some(),
                    s = ur.next() => s.is_some(),
                    else => false,
                };
                if !changed || tx.send(()).await.is_err() {
                    break; // conexão caiu ou ninguém escutando mais
                }
            }
        });

        Some(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test ao vivo contra o systemd --user real desta máquina.
    /// Ignorado por padrão (CI pode não ter session bus). Rode com:
    /// `cargo test -- --ignored systemd_live`
    #[tokio::test]
    #[ignore]
    async fn systemd_live_lists_units() {
        let provider = SystemdUserProvider::connect()
            .await
            .expect("conectar ao session bus");
        assert!(provider.available().await);
        let jobs = provider.list().await.expect("listar units");
        eprintln!("--- {} jobs do systemd --user ---", jobs.len());
        for j in jobs.iter().take(40) {
            eprintln!(
                "{:?} {:<40} state={:?} enabled={} sched={:?}",
                j.kind, j.name, j.state, j.enabled, j.schedule
            );
        }
        assert!(!jobs.is_empty(), "deveria haver ao menos uma user-unit");
    }

    /// Lê CPU/memória de um serviço ativo (duas amostras para obter CPU%).
    /// `cargo test -- --ignored metrics_live --nocapture`
    #[tokio::test]
    #[ignore]
    async fn metrics_live() {
        let provider = SystemdUserProvider::connect().await.unwrap();
        let jobs = provider.list().await.unwrap();
        let active = jobs
            .into_iter()
            .find(|j| matches!(j.state, JobState::Active) && j.local_id.ends_with(".service"))
            .expect("precisa de um .service ativo para o teste");
        eprintln!("medindo: {}", active.local_id);

        let r1 = provider.metrics(&active.local_id).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let r2 = provider.metrics(&active.local_id).await.unwrap();

        eprintln!(
            "1ª amostra: cpu={:?} mem={:?} pids={:?}",
            r1.cpu_pct, r1.mem_bytes, r1.pids
        );
        eprintln!(
            "2ª amostra: cpu={:?} mem={:?} pids={:?}",
            r2.cpu_pct, r2.mem_bytes, r2.pids
        );
        assert!(
            r2.mem_bytes.is_some(),
            "memória deveria estar disponível (cgroup v2)"
        );
        assert!(r1.cpu_pct.is_none(), "1ª amostra não tem delta → CPU None");
        assert!(
            r2.cpu_pct.is_some(),
            "2ª amostra deveria ter CPU% calculado"
        );
    }

    /// Detalhe + logs de um serviço real.
    /// `cargo test -- --ignored detail_live --nocapture`
    #[tokio::test]
    #[ignore]
    async fn detail_live() {
        let provider = SystemdUserProvider::connect().await.unwrap();
        let jobs = provider.list().await.unwrap();
        let svc = jobs
            .iter()
            .find(|j| j.local_id.ends_with(".service"))
            .expect("precisa de um .service");
        eprintln!("detalhe de: {}", svc.local_id);

        let d = provider.detail(&svc.local_id).await.unwrap();
        eprintln!(
            "command={:?}\nfragment={:?}\nexit_code={:?} reason={:?} since={:?}",
            d.command, d.fragment_path, d.exit_code, d.exit_reason, d.since
        );
        assert!(d.fragment_path.is_some(), "deveria achar o FragmentPath");

        let logs = provider.logs(&svc.local_id, 5).await.unwrap();
        eprintln!("--- {} linhas de log ---", logs.len());
        for l in &logs {
            eprintln!("{l}");
        }
    }
}
