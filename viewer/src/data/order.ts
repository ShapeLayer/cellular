/**
 * Ordering commits for display.
 *
 * Sorting by committer time alone is not enough: rebases, imports and scripted
 * commits routinely produce identical or non-monotonic timestamps, and then a
 * time-sorted list puts a parent before its child and the timeline graph falls
 * apart into a staircase. Ordering topologically — every commit before its
 * parents, time only as a tiebreak — keeps the graph readable whatever the
 * timestamps say.
 */

import type { Snapshot } from '../model';

/** Newest first: later commits, then their parents. */
function newerFirst(left: Snapshot, right: Snapshot): number {
  return right.commitTime - left.commitTime || right.oid.localeCompare(left.oid);
}

/**
 * Insert into a list that is kept sorted by {@link newerFirst}.
 *
 * The item belongs after everything {@link newerFirst} ranks ahead of it, so a
 * hit walks the search to the right. Reading the comparison the other way puts
 * every newly ready commit at the wrong end of the queue, and the walk then
 * empties out the oldest branch tips before it ever reaches the newest one.
 */
function insertSorted(list: Snapshot[], item: Snapshot): void {
  let low = 0;
  let high = list.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (newerFirst(list[middle], item) <= 0) low = middle + 1;
    else high = middle;
  }
  list.splice(low, 0, item);
}

/**
 * Commits newest first, with every commit placed before its parents.
 *
 * Among commits that are free to go next, the most recent one wins, so an
 * ordinary history still reads chronologically.
 */
export function orderTopological(snapshots: Snapshot[]): Snapshot[] {
  const present = new Map(snapshots.map((snapshot) => [snapshot.oid, snapshot]));

  // How many commits still to be emitted claim this one as a parent.
  const pendingChildren = new Map<string, number>();
  for (const snapshot of snapshots) {
    for (const parent of snapshot.parents) {
      if (present.has(parent)) {
        pendingChildren.set(parent, (pendingChildren.get(parent) ?? 0) + 1);
      }
    }
  }

  const ready = snapshots
    .filter((snapshot) => (pendingChildren.get(snapshot.oid) ?? 0) === 0)
    .sort(newerFirst);

  const ordered: Snapshot[] = [];
  const emitted = new Set<string>();
  while (ready.length > 0) {
    const next = ready.shift() as Snapshot;
    ordered.push(next);
    emitted.add(next.oid);
    for (const parent of next.parents) {
      const snapshot = present.get(parent);
      if (!snapshot || emitted.has(parent)) continue;
      const left = (pendingChildren.get(parent) ?? 0) - 1;
      pendingChildren.set(parent, left);
      if (left <= 0) insertSorted(ready, snapshot);
    }
  }

  // Git cannot produce a cycle, but a hand-made index could; never drop a
  // commit because of one.
  if (ordered.length < snapshots.length) {
    const seen = new Set(ordered.map((snapshot) => snapshot.oid));
    ordered.push(...snapshots.filter((snapshot) => !seen.has(snapshot.oid)).sort(newerFirst));
  }
  return ordered;
}

/** The same order, oldest first. */
export function orderOldestFirst(snapshots: Snapshot[]): Snapshot[] {
  return orderTopological(snapshots).reverse();
}
