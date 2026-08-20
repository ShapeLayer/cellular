/** Shapes the viewer works with, mirroring the runner's index records. */

export interface Counts {
  files: number;
  lines: number;
  chars: number;
}

export interface ModuleStats {
  /** Repository-relative module path, `.` for the repository root. */
  path: string;
  totals: Counts;
  /** Language name to counts; empty when the runner skipped that metric. */
  languages: Map<string, Counts>;
}

export interface Snapshot {
  oid: string;
  parents: string[];
  /** Branch and tag names pointing at this commit. */
  refs: string[];
  /** Committer time, seconds since the Unix epoch. */
  commitTime: number;
  /** Committer time zone offset in seconds east of UTC. */
  tzOffset: number;
  /** The commit spec that produced this record, empty once it moved on. */
  spec: string;
  summary: string;
  author: string;
  indexDepth: number;
  metrics: number;
  modules: ModuleStats[];
}

export const METRIC_LINES = 1 << 0;
export const METRIC_CHARS = 1 << 1;
export const METRIC_LANGUAGES = 1 << 2;

/** What a treemap block's area is proportional to. */
export type Metric = 'lines' | 'chars' | 'files';

export const METRIC_LABELS: Record<Metric, string> = {
  lines: 'Lines',
  chars: 'Characters',
  files: 'Files',
};

/**
 * Which metrics an index can actually answer for.
 *
 * The runner records what it collected in `metrics`; a build made with
 * `-m lines` stores zeros for characters, which would otherwise show up as a
 * blank treemap with no explanation. File counts always come along.
 */
export function availableMetrics(snapshots: Snapshot[]): Metric[] {
  const available: Metric[] = ['files'];
  if (snapshots.some((snapshot) => (snapshot.metrics & METRIC_LINES) !== 0)) available.unshift('lines');
  if (snapshots.some((snapshot) => (snapshot.metrics & METRIC_CHARS) !== 0)) {
    available.splice(available.length - 1, 0, 'chars');
  }
  return available;
}

export function languagesAvailable(snapshots: Snapshot[]): boolean {
  return snapshots.some((snapshot) => (snapshot.metrics & METRIC_LANGUAGES) !== 0);
}

export function measure(counts: Counts, metric: Metric): number {
  return counts[metric];
}

export function snapshotTotal(snapshot: Snapshot, metric: Metric): number {
  let total = 0;
  for (const module of snapshot.modules) total += measure(module.totals, metric);
  return total;
}

/** The language that accounts for most of a module, by the chosen metric. */
export function dominantLanguage(module: ModuleStats, metric: Metric): string | null {
  let best: string | null = null;
  let bestValue = -1;
  for (const [name, counts] of module.languages) {
    const value = measure(counts, metric);
    if (value > bestValue) {
      bestValue = value;
      best = name;
    }
  }
  return best;
}

export function shortOid(oid: string): string {
  return oid.slice(0, 10);
}

export function commitDate(snapshot: Snapshot): Date {
  return new Date(snapshot.commitTime * 1000);
}

/** Commit time rendered in the author's own time zone. */
export function formatCommitTime(snapshot: Snapshot): string {
  const shifted = new Date((snapshot.commitTime + snapshot.tzOffset) * 1000);
  const pad = (value: number) => String(value).padStart(2, '0');
  const offsetMinutes = Math.round(snapshot.tzOffset / 60);
  const sign = offsetMinutes < 0 ? '-' : '+';
  const absolute = Math.abs(offsetMinutes);
  return (
    `${shifted.getUTCFullYear()}-${pad(shifted.getUTCMonth() + 1)}-${pad(shifted.getUTCDate())} ` +
    `${pad(shifted.getUTCHours())}:${pad(shifted.getUTCMinutes())} ` +
    `${sign}${pad(Math.floor(absolute / 60))}:${pad(absolute % 60)}`
  );
}
