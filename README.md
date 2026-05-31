<div align="center">

# Maestro

**Cockpit de jobs de background — veja e controle seus serviços, timers e daemons sem CLI.**

</div>

Maestro reúne tudo que roda em segundo plano na sua máquina numa interface única,
dark-first: estado ao vivo, próximo/último disparo, autostart e controle direto
(iniciar, parar, reiniciar, habilitar, disparar agora) — sem decorar comandos.

Mostra **apenas os seus serviços** — não o plumbing do sistema operacional.

## Estado atual (v1)

- **Linux / `systemd --user`** — services e timers definidos pelo usuário.
- Overview com busca, filtros (todos / falhas / ativos / agendados) e auto-refresh.
- Controle de ciclo de vida via D-Bus tipado (zbus), **sem shell**.

Roadmap: métricas de CPU/memória ao vivo, painel de detalhe com logs (journald),
notificações de falha, e novos providers (Docker, cron, launchd no macOS, Task
Scheduler no Windows) — todos atrás da mesma abstração `JobProvider`.

## Instalação

Baixe o pacote da sua distro na página de
[**Releases**](https://github.com/renatoacj/maestro/releases/latest).

| Distro | Arquivo | Instalar |
|---|---|---|
| **Qualquer distro** | `Maestro_0.1.0_amd64.AppImage` | `chmod +x Maestro_*.AppImage && ./Maestro_*.AppImage` |
| **Debian / Ubuntu / Mint** | `Maestro_0.1.0_amd64.deb` | `sudo apt install ./Maestro_0.1.0_amd64.deb` |
| **Fedora / RHEL / openSUSE** | `Maestro-0.1.0-1.x86_64.rpm` | `sudo dnf install ./Maestro-0.1.0-1.x86_64.rpm` |
| **Arch / CachyOS / Manjaro** | `packaging/PKGBUILD` | `cd packaging && makepkg -si` |

O AppImage é autocontido (não precisa instalar nada). Os pacotes `.deb`/`.rpm`/Arch
puxam as dependências de runtime automaticamente: `webkit2gtk-4.1`, `gtk3` e
`libayatana-appindicator` (ícone na bandeja).

Verifique a integridade com o `SHA256SUMS.txt` anexado ao release:

```bash
sha256sum -c SHA256SUMS.txt
```

## Desenvolvimento

Requer Rust, Node 20+ e as libs do Tauri (`webkit2gtk-4.1`, `gtk3`,
`libayatana-appindicator` para o ícone na bandeja).

```bash
npm install
npm run tauri dev      # janela de desenvolvimento
npm run tauri build    # gera o binário e os instaladores
cargo test --manifest-path src-tauri/Cargo.toml
```

## Arquitetura

A trait [`JobProvider`](src-tauri/src/provider/mod.rs) abstrai cada mecanismo
(systemd, Docker, cron, launchd, Task Scheduler). O núcleo nunca muda — suportar um
OS novo é escrever um provider. Backend em **Rust + Tauri v2**; frontend em
**Svelte 5**. Design completo em
[docs/superpowers/specs/](docs/superpowers/specs/2026-05-30-maestro-design.md).

## Stack

Rust · Tauri v2 · zbus (D-Bus) · Svelte 5 · SvelteKit · TypeScript
