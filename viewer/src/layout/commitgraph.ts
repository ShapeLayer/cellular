/**
 * Lane assignment for the timeline's commit graph.
 *
 * Commits are drawn newest first, one per row, and every commit sits in a lane
 * — a vertical column a branch keeps for its whole life. A lane is never
 * shifted sideways once handed out: an edge records the lane its parent will
 * occupy many rows later, so a lane that moved would leave that line ending in
 * the wrong column.
 *
 * The first parent continues the lane the commit is already in, so mainline
 * stays put, and the other parents of a merge open lanes beside it. A line
 * between two lanes runs straight down one of them and turns a rounded corner
 * at the end that sits in the other, which is the shape GitKraken's commit
 * graph draws.
 */

import type { Snapshot } from '../model';

export interface GraphNode {
  snapshot: Snapshot;
  /** Position in the list, 0 being the newest commit. */
  row: number;
  lane: number;
  merge: boolean;
  /** True when this commit has parents the index does not hold. */
  truncated: boolean;
}

export interface GraphEdge {
  fromRow: number;
  fromLane: number;
  toRow: number;
  toLane: number;
  /**
   * The lane the straight part of the line runs down. It is the parent's lane
   * for a branch opening below a merge, and the commit's own lane for a branch
   * ending at the line it grew out of; the corner is turned at the other end.
   */
  runsIn: number;
}

export interface CommitGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  /** How many lanes the graph needs at its widest. */
  lanes: number;
}

export function buildCommitGraph(snapshots: Snapshot[]): CommitGraph {
  // The loader hands over an index ordered oldest first, with every commit
  // already placed after its children, so reversing gives the display order.
  const order = [...snapshots].reverse();
  const rowOf = new Map(order.map((snapshot, row) => [snapshot.oid, row]));

  /**
   * Per lane, the last row it is still spoken for — either because a commit
   * down there will land in it, or because a line is running through it on its
   * way to one. A lane is free again once that row is above the current one.
   */
  const heldUntil: number[] = [];
  /** Lanes already promised to a commit further down the list. */
  const reserved = new Map<string, number>();
  const nodes: GraphNode[] = [];
  const edges: GraphEdge[] = [];

  /**
   * A free lane, looked for to the right of `from` first so a merged branch
   * opens beside the merge that took it in.
   *
   * Failing that it falls back to the lanes on the left rather than opening a
   * new column. Merges nest, and each one asking for a lane strictly to the
   * right of a lane that was itself won that way walks the graph off to the
   * right for good: this history needs nine lanes at its busiest and drifted
   * to forty-five without the fallback.
   */
  const take = (from: number, row: number): number => {
    for (let lane = from; lane < heldUntil.length; lane += 1) {
      if (heldUntil[lane] < row) return lane;
    }
    for (let lane = 0; lane < from && lane < heldUntil.length; lane += 1) {
      if (heldUntil[lane] < row) return lane;
    }
    heldUntil.push(row);
    return heldUntil.length - 1;
  };

  const hold = (lane: number, until: number): void => {
    heldUntil[lane] = Math.max(heldUntil[lane], until);
  };

  order.forEach((snapshot, row) => {
    let lane = reserved.get(snapshot.oid);
    if (lane === undefined) {
      // Nothing below pointed here, so this is the tip of a branch: it starts
      // a lane of its own in the leftmost free column.
      lane = take(0, row);
    } else {
      reserved.delete(snapshot.oid);
    }
    // The lane has arrived; from here it only lives on if a parent carries it
    // further down the list.
    heldUntil[lane] = row;

    const parents = snapshot.parents.filter((parent) => rowOf.has(parent));
    parents.forEach((parent, ancestry) => {
      const toRow = rowOf.get(parent) as number;
      let toLane = reserved.get(parent);
      if (toLane === undefined) {
        // The first parent stays in this lane; a merge's other parents get
        // a lane of their own, beside it wherever there is room.
        toLane = ancestry === 0 ? lane : take(lane + 1, row);
        reserved.set(parent, toLane);
      }
      hold(toLane, toRow);
      // A first parent that lives somewhere else leaves this lane behind, so
      // the line can run down it and turn in at the bottom. Any other parent
      // has to keep clear of it: the first parent's line is already there.
      const runsIn = ancestry === 0 ? lane : toLane;
      // Hold the whole run of the line, or another branch could move in
      // underneath a line that is still on its way down.
      hold(runsIn, toRow);
      edges.push({ fromRow: row, fromLane: lane, toRow, toLane, runsIn });
    });

    nodes.push({
      snapshot,
      row,
      lane,
      merge: snapshot.parents.length > 1,
      truncated: parents.length === 0 && snapshot.parents.length > 0,
    });
  });

  return { nodes, edges, lanes: Math.max(1, heldUntil.length) };
}
