// Formatadores de exibição — tempo relativo e bytes.

export function relativeTime(epochSecs: number | null): string {
  if (epochSecs == null) return "—";
  const now = Date.now() / 1000;
  const delta = epochSecs - now; // futuro positivo, passado negativo
  const abs = Math.abs(delta);
  const fut = delta >= 0;

  const units: [number, string][] = [
    [60, "s"],
    [60, "min"],
    [24, "h"],
    [7, "d"],
    [4.345, "sem"],
    [12, "mês"],
    [Number.POSITIVE_INFINITY, "ano"],
  ];

  let value = abs;
  let label = "s";
  for (const [step, name] of units) {
    if (value < step) {
      label = name;
      break;
    }
    value /= step;
    label = name;
  }
  const n = Math.round(value);
  return fut ? `em ${n}${label}` : `há ${n}${label}`;
}

export function formatBytes(bytes: number | null): string {
  if (bytes == null) return "—";
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const v = bytes / Math.pow(1024, i);
  return `${v.toFixed(v < 10 && i > 0 ? 1 : 0)} ${units[i]}`;
}
