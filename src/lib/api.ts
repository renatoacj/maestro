// Espelho tipado do modelo Rust + wrappers sobre o IPC do Tauri.
// Esta é a única superfície pela qual o frontend fala com o núcleo.

import { invoke } from "@tauri-apps/api/core";

export type JobKind = "service" | "scheduled" | "container";
export type JobState =
  | "active"
  | "inactive"
  | "failed"
  | "activating"
  | "deactivating"
  | "unknown";
export type Health = "ok" | "degraded" | "failed";
export type Action =
  | "start"
  | "stop"
  | "restart"
  | "enable"
  | "disable"
  | "trigger_now";

export interface Schedule {
  nextRun: number | null; // epoch (s)
  lastRun: number | null; // epoch (s)
}

export interface Resources {
  cpuPct: number | null;
  memBytes: number | null;
  pids: number[];
}

export interface Job {
  id: string;
  provider: string;
  localId: string;
  kind: JobKind;
  name: string;
  description: string;
  command: string | null;
  state: JobState;
  enabled: boolean;
  schedule: Schedule | null;
  resources: Resources;
  health: Health;
}

export const listJobs = () => invoke<Job[]>("list_jobs");

export const controlJob = (id: string, action: Action) =>
  invoke<void>("control_job", { id, action });

export const jobMetrics = (id: string) => invoke<Resources>("job_metrics", { id });
