/**
 * The timeline control: a git graph pinned to the right of the window.
 *
 * Commits are drawn newest first. Lanes come from the parent links the runner
 * stores, so branches and merges keep their shape instead of collapsing into a
 * straight list. Commits whose parents are not in the index end in a dashed
 * stub, which is what a sparse index looks like.
 */

import { formatCommitTime, shortOid, type Snapshot } from '../model';

const SVG_NS = 'http://www.w3.org/2000/svg';
const ROW = 26;
const LANE = 14;
const GRAPH_LEFT = 14;
const TOP = 10;

interface Row {
  snapshot: Snapshot;
  lane: number;
  edges: Array<{ toIndex: number; toLane: number }>;
  /** True when history continues past the commits held in this index. */
  truncated: boolean;
}

export interface CommitHoverDetail {
  snapshot: Snapshot | null;
  clientX: number;
  clientY: number;
}

const TEMPLATE = `
  <style>
    :host {
      position: fixed;
      right: 12px;
      top: 50%;
      transform: translateY(-50%);
      z-index: 30;
      width: 292px;
      max-height: 80vh;
      display: flex;
      flex-direction: column;
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      box-shadow: var(--shadow-sm);
      font-family: var(--font);
      overflow: hidden;
    }
    :host([hidden]) { display: none; }
    header {
      display: flex;
      align-items: baseline;
      justify-content: space-between;
      gap: 8px;
      padding: 8px 10px;
      border-bottom: 1px solid var(--border);
      font-size: 12px;
      font-weight: 600;
      color: var(--text);
      flex: none;
    }
    header .count { font-weight: 400; color: var(--text-muted); }
    .scroll { overflow-y: auto; overflow-x: hidden; flex: 1 1 auto; }
    .hint {
      padding: 6px 10px 8px;
      border-top: 1px solid var(--border);
      color: var(--text-faint);
      font-size: 11px;
      flex: none;
    }
    svg { display: block; user-select: none; touch-action: none; }
    .row-hit { fill: transparent; cursor: pointer; }
    .row-hit:hover { fill: var(--surface-hover); }
    .row-hit.selected { fill: var(--accent-soft); }
    .edge { fill: none; stroke: var(--border-strong); stroke-width: 1.5; pointer-events: none; }
    .edge.stub { stroke-dasharray: 3 3; }
    .dot { fill: var(--surface); stroke: var(--border-strong); stroke-width: 1.5; }
    .dot.selected { fill: var(--accent); stroke: var(--accent); }
    .oid { font-family: var(--mono); font-size: 10.5px; fill: var(--text-muted); }
    .oid.selected { fill: var(--accent); font-weight: 600; }
    .summary { font-size: 11.5px; fill: var(--text); }
    .refs { font-size: 10px; fill: var(--accent); font-family: var(--mono); }
    .empty { padding: 14px 12px; color: var(--text-muted); font-size: 12px; }
  </style>
  <header><span>Timeline</span><span class="count"></span></header>
  <div class="scroll"></div>
  <div class="hint">Click to select · drag to scrub · ctrl+click to add · shift+click for a range</div>
`;

export class CellularTimeline extends HTMLElement {
  private snapshots: Snapshot[] = [];
  private selected = new Set<string>();
  private anchor: string | null = null;
  private rows: Row[] = [];
  private list!: HTMLElement;
  private count!: HTMLElement;
  private scrubbing = false;
  /** The commit the list was last scrolled to, so it only moves on a change. */
  private scrolledTo: string | null = null;

  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = TEMPLATE;
    this.list = root.querySelector('.scroll') as HTMLElement;
    // A pointer released outside the graph must also end a scrub.
    window.addEventListener('pointerup', () => {
      this.scrubbing = false;
    });
    this.count = root.querySelector('.count') as HTMLElement;
    this.render();
  }

  setData(snapshots: Snapshot[], selected: string[]): void {
    this.snapshots = snapshots;
    this.selected = new Set(selected);
    this.keepAnchor(selected);
    this.rows = layout(snapshots);
    this.render();
  }

  setSelection(selected: string[]): void {
    this.selected = new Set(selected);
    this.keepAnchor(selected);
    this.render();
  }

  /**
   * The anchor is where a shift range starts. It survives a range selection,
   * and falls back to the current selection whenever it is no longer part of
   * it — including the first selection after loading a file.
   */
  private keepAnchor(selected: string[]): void {
    if (!this.anchor || !this.selected.has(this.anchor)) this.anchor = selected[0] ?? null;
  }

  private emitSelection(oids: string[]): void {
    this.dispatchEvent(
      new CustomEvent<{ oids: string[] }>('commit-select', {
        detail: { oids },
        bubbles: true,
        composed: true,
      }),
    );
  }

  private emitHover(snapshot: Snapshot | null, clientX: number, clientY: number): void {
    this.dispatchEvent(
      new CustomEvent<CommitHoverDetail>('commit-hover', {
        detail: { snapshot, clientX, clientY },
        bubbles: true,
        composed: true,
      }),
    );
  }

  private select(index: number, event: PointerEvent | MouseEvent): void {
    const row = this.rows[index];
    if (!row) return;
    const oid = row.snapshot.oid;
    const order = this.rows.map((entry) => entry.snapshot.oid);

    if (event.shiftKey && this.anchor) {
      const from = order.indexOf(this.anchor);
      const to = index;
      if (from !== -1) {
        const [low, high] = from <= to ? [from, to] : [to, from];
        this.emitSelection(order.slice(low, high + 1));
        return;
      }
    }
    if (event.ctrlKey || event.metaKey) {
      const next = new Set(this.selected);
      if (next.has(oid)) next.delete(oid);
      else next.add(oid);
      this.anchor = oid;
      // Keep the timeline order rather than the order of clicks.
      this.emitSelection(order.filter((entry) => next.has(entry)));
      return;
    }
    this.anchor = oid;
    this.emitSelection([oid]);
  }

  private render(): void {
    if (!this.list) return;
    this.count.textContent = this.snapshots.length > 0 ? `${this.snapshots.length} commits` : '';
    this.list.textContent = '';

    if (this.rows.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty';
      empty.textContent = 'No commits loaded.';
      this.list.append(empty);
      return;
    }

    const lanes = Math.max(1, ...this.rows.map((row) => row.lane + 1));
    const graphWidth = GRAPH_LEFT + lanes * LANE;
    const width = 290;
    const height = TOP * 2 + this.rows.length * ROW;

    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('width', String(width));
    svg.setAttribute('height', String(height));
    svg.setAttribute('viewBox', `0 0 ${width} ${height}`);

    const laneX = (lane: number) => GRAPH_LEFT + lane * LANE;
    const rowY = (index: number) => TOP + index * ROW + ROW / 2;

    // Row backgrounds must sit behind the graph. Otherwise a selected row's
    // highlight hides any branch line that crosses it.
    this.rows.forEach((row, index) => {
      const selected = this.selected.has(row.snapshot.oid);
      const hit = document.createElementNS(SVG_NS, 'rect');
      hit.setAttribute('class', `row-hit${selected ? ' selected' : ''}`);
      hit.setAttribute('x', '0');
      hit.setAttribute('y', String(TOP + index * ROW));
      hit.setAttribute('width', String(width));
      hit.setAttribute('height', String(ROW));
      hit.addEventListener('pointerdown', (event) => {
        this.scrubbing = !event.shiftKey && !event.ctrlKey && !event.metaKey;
        this.select(index, event);
      });
      hit.addEventListener('pointerenter', (event) => {
        this.emitHover(row.snapshot, event.clientX, event.clientY);
        // Dragging across the graph walks the selection along with the pointer.
        if (this.scrubbing) this.emitSelection([row.snapshot.oid]);
      });
      hit.addEventListener('pointermove', (event) =>
        this.emitHover(row.snapshot, event.clientX, event.clientY),
      );
      if (selected) hit.dataset.selected = 'true';
      const stamp = document.createElementNS(SVG_NS, 'title');
      stamp.textContent = formatCommitTime(row.snapshot);
      hit.append(stamp);
      svg.append(hit);
    });

    // Edges come after backgrounds, so they remain visible over a selection.
    this.rows.forEach((row, index) => {
      const x1 = laneX(row.lane);
      const y1 = rowY(index);
      for (const edge of row.edges) {
        const x2 = laneX(edge.toLane);
        const y2 = rowY(edge.toIndex);
        const path = document.createElementNS(SVG_NS, 'path');
        path.setAttribute('class', 'edge');
        path.setAttribute(
          'd',
          x1 === x2
            ? `M ${x1} ${y1} L ${x2} ${y2}`
            : `M ${x1} ${y1} C ${x1} ${y1 + ROW * 0.6}, ${x2} ${y2 - ROW * 0.6}, ${x2} ${y2}`,
        );
        svg.append(path);
      }
      if (row.truncated) {
        const stub = document.createElementNS(SVG_NS, 'path');
        stub.setAttribute('class', 'edge stub');
        stub.setAttribute('d', `M ${x1} ${y1} L ${x1} ${y1 + ROW * 0.7}`);
        svg.append(stub);
      }
    });

    this.rows.forEach((row, index) => {
      const selected = this.selected.has(row.snapshot.oid);
      const y = rowY(index);

      const dot = document.createElementNS(SVG_NS, 'circle');
      dot.setAttribute('class', `dot${selected ? ' selected' : ''}`);
      dot.setAttribute('cx', String(laneX(row.lane)));
      dot.setAttribute('cy', String(y));
      dot.setAttribute('r', selected ? '5' : '4');
      dot.setAttribute('pointer-events', 'none');
      svg.append(dot);

      const oid = document.createElementNS(SVG_NS, 'text');
      oid.setAttribute('class', `oid${selected ? ' selected' : ''}`);
      oid.setAttribute('x', String(graphWidth + 6));
      oid.setAttribute('y', String(y - 3));
      oid.setAttribute('pointer-events', 'none');
      oid.textContent = shortOid(row.snapshot.oid).slice(0, 8);
      svg.append(oid);

      if (row.snapshot.refs.length > 0) {
        const refs = document.createElementNS(SVG_NS, 'text');
        refs.setAttribute('class', 'refs');
        refs.setAttribute('x', String(graphWidth + 60));
        refs.setAttribute('y', String(y - 3));
        refs.setAttribute('pointer-events', 'none');
        refs.textContent = row.snapshot.refs.join(', ');
        svg.append(refs);
      }

      const summary = document.createElementNS(SVG_NS, 'text');
      summary.setAttribute('class', 'summary');
      summary.setAttribute('x', String(graphWidth + 6));
      summary.setAttribute('y', String(y + 9));
      summary.setAttribute('pointer-events', 'none');
      summary.textContent = row.snapshot.summary || '(no message)';
      svg.append(summary);
      clampText(summary, width - graphWidth - 12);

    });

    svg.addEventListener('pointerleave', () => {
      this.scrubbing = false;
      this.emitHover(null, 0, 0);
    });
    svg.addEventListener('pointerup', () => {
      this.scrubbing = false;
    });

    this.list.append(svg);
    this.revealSelection(svg);
  }

  /**
   * Bring the first selected commit into view. With a long history a scrub or
   * a range selection easily lands off screen.
   */
  private revealSelection(svg: SVGSVGElement): void {
    const first = this.rows.findIndex((row) => this.selected.has(row.snapshot.oid));
    if (first < 0) return;
    const oid = this.rows[first].snapshot.oid;
    // Only move when the selection changed, or the list would fight the user
    // scrolling through it.
    if (oid === this.scrolledTo) return;
    this.scrolledTo = oid;

    const target = svg.querySelector('[data-selected]');
    target?.scrollIntoView({ block: 'nearest' });
  }
}

/** Trim an SVG text node until it fits, ending with an ellipsis. */
function clampText(node: SVGTextElement, maxWidth: number): void {
  const original = node.textContent ?? '';
  if (node.getComputedTextLength?.() <= maxWidth) return;
  let text = original;
  while (text.length > 1 && node.getComputedTextLength() > maxWidth) {
    text = text.slice(0, -1);
    node.textContent = `${text}…`;
  }
}

/** Assign a lane to every commit, newest first. */
function layout(snapshots: Snapshot[]): Row[] {
  // The loader already ordered these topologically, oldest first, so the graph
  // holds up even when commits share a timestamp.
  const ordered = [...snapshots].reverse();
  const indexOf = new Map(ordered.map((snapshot, index) => [snapshot.oid, index]));

  // Each slot holds the commit the lane is currently waiting to reach.
  const lanes: Array<string | null> = [];
  const firstFree = (): number => {
    const free = lanes.indexOf(null);
    if (free !== -1) return free;
    lanes.push(null);
    return lanes.length - 1;
  };

  return ordered.map((snapshot) => {
    let lane = lanes.indexOf(snapshot.oid);
    if (lane === -1) lane = firstFree();
    lanes[lane] = null;

    const parents = snapshot.parents.filter((parent) => indexOf.has(parent));
    const edges = parents.map((parent, order) => {
      const existing = lanes.indexOf(parent);
      let target: number;
      if (existing !== -1) {
        target = existing;
      } else if (order === 0 && lanes[lane] === null) {
        target = lane;
        lanes[lane] = parent;
      } else {
        target = firstFree();
        lanes[target] = parent;
      }
      return { toIndex: indexOf.get(parent) as number, toLane: target };
    });

    return {
      snapshot,
      lane,
      edges,
      truncated: parents.length === 0 && snapshot.parents.length > 0,
    };
  });
}

customElements.define('cellular-timeline', CellularTimeline);
