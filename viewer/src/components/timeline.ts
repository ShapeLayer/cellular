/**
 * The timeline control: a commit graph pinned to the right of the window.
 *
 * Commits are drawn newest first, one row each. Lanes come from the parent
 * links the runner stores, so branches and merges keep their shape instead of
 * collapsing into a straight list; `layout/commitgraph` works them out. This
 * file is only the drawing: rows are ordinary elements, so a long message
 * simply ends in an ellipsis, and the graph is a single overlaid SVG that owns
 * every line and node.
 *
 * Lines follow the shape GitKraken's commit graph uses. A line between two
 * lanes runs straight down the one further right and turns a rounded corner at
 * the end sitting in the other, so a branch leaves its merge sideways at the
 * top and rejoins its parent line at the bottom. Commits whose parents are not
 * in the index end in a dashed stub, which is what a sparse index looks like.
 */

import { buildCommitGraph, type CommitGraph } from '../layout/commitgraph';
import type { Snapshot } from '../model';

const SVG_NS = 'http://www.w3.org/2000/svg';

/** Height of one commit row. */
const ROW = 26;
/** Preferred distance between lanes, and how far they may be squeezed. */
const LANE = 14;
const LANE_MIN = 9;
/** Space left of the first lane, and again right of the last one. */
const GRAPH_PAD = 12;
/** The graph gives way to the message once the rows get this tight. */
const TEXT_MIN = 150;
/** Radius of the turn a line makes on its way into another lane. */
const CORNER = 10;
/** How far the stub of a parent outside the index hangs below its commit. */
const STUB = 9;
/**
 * Refs past this many collapse into a `+n` chip. The panel is narrow, and the
 * commit's tooltip lists all of them anyway.
 */
const REF_LIMIT = 1;

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
    :host([minimized]) {
      top: auto;
      bottom: 12px;
      transform: none;
      width: auto;
      max-height: none;
    }
    header {
      display: flex;
      align-items: center;
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
    .title { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
    .controls { display: flex; align-items: center; margin-left: auto; }
    .control {
      appearance: none;
      display: grid;
      place-items: center;
      width: 22px;
      height: 22px;
      margin: -4px 0 -4px 2px;
      padding: 0;
      border: 0;
      border-radius: 4px;
      background: transparent;
      color: var(--text-muted);
      font: inherit;
      font-size: 16px;
      line-height: 1;
      cursor: pointer;
    }
    .control:hover { background: var(--surface-hover); color: var(--text); }
    .close { position: relative; font-size: 0; }
    .close::before, .close::after {
      content: '';
      position: absolute;
      inset: 0;
      margin: auto;
      width: 10px;
      height: 1.5px;
      border-radius: 1px;
      background: currentColor;
    }
    .close::before { transform: rotate(45deg); }
    .close::after { transform: rotate(-45deg); }
    .close:hover { background: var(--danger); color: var(--surface); }
    :host([minimized]) .scroll,
    :host([minimized]) .hint { display: none; }
    .scroll {
      flex: 1 1 auto;
      overflow: auto;
      overscroll-behavior: contain;
      padding: 6px 0;
    }
    .hint {
      padding: 6px 10px 8px;
      border-top: 1px solid var(--border);
      color: var(--text-faint);
      font-size: 11px;
      flex: none;
    }
    .empty { padding: 14px 12px; color: var(--text-muted); font-size: 12px; }

    /* The graph is laid over the rows: its lines have to stay visible across a
       selected row, and its nodes have to stay on top of the lines. */
    .body { position: relative; user-select: none; }
    .lines { position: absolute; left: 0; top: 0; pointer-events: none; }

    .row {
      --row-bg: transparent;
      position: relative;
      display: flex;
      align-items: center;
      gap: 6px;
      height: ${ROW}px;
      padding-left: calc(var(--graph, 0px) + 6px);
      padding-right: 9px;
      background: var(--row-bg);
      cursor: pointer;
      white-space: nowrap;
    }
    .row:hover { --row-bg: var(--surface-hover); }
    .row.selected { --row-bg: var(--accent-soft); }

    .refs {
      display: flex;
      gap: 4px;
      flex: 0 1 auto;
      /* Wide enough that a trimmed branch name is still worth reading, and
         never so wide that it takes the message's room. */
      min-width: 56px;
      max-width: 55%;
      overflow: hidden;
    }
    .chip {
      box-sizing: border-box;
      flex: 0 1 auto;
      min-width: 0;
      max-width: 132px;
      white-space: nowrap;
      height: 16px;
      padding: 0 5px;
      border: 1px solid var(--accent);
      border-radius: 999px;
      color: var(--accent);
      font-family: var(--mono);
      font-size: 9.5px;
      line-height: 14px;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    /* A remote is a copy of a branch that lives elsewhere; it should not read
       as loudly as the local one standing next to it. */
    .chip.remote { border-color: var(--border-strong); color: var(--text-muted); }
    .chip.more { flex: none; max-width: none; border-style: dashed; }

    .message {
      flex: 1 1 auto;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      font-size: 11.5px;
      color: var(--text);
    }
    .message.empty-message { color: var(--text-faint); font-style: italic; }
    .row.selected .message { font-weight: 600; }
    .oid {
      flex: none;
      font-family: var(--mono);
      font-size: 10px;
      color: var(--text-faint);
    }
    .row.selected .oid { color: var(--accent); }
  </style>
  <header>
    <span class="title"><span>Timeline</span><span class="count"></span></span>
    <span class="controls">
      <button class="control minimize" type="button" aria-label="Minimize timeline" title="Minimize timeline">−</button>
      <button class="control close" type="button" aria-label="Close timeline" title="Close timeline">×</button>
    </span>
  </header>
  <div class="scroll"><div class="body"></div></div>
  <div class="hint">Click to select · drag to scrub · ctrl+click to add · shift+click for a range</div>
`;

export class CellularTimeline extends HTMLElement {
  private snapshots: Snapshot[] = [];
  private graph: CommitGraph = { nodes: [], edges: [], lanes: 1 };
  private selected = new Set<string>();
  private anchor: string | null = null;
  private viewport!: HTMLElement;
  private body!: HTMLElement;
  private count!: HTMLElement;
  private minimizeButton!: HTMLButtonElement;
  private rows: HTMLElement[] = [];
  private lines: SVGSVGElement | null = null;
  private scrubbing = false;
  private resizes: ResizeObserver | null = null;
  /** Width the graph was last drawn for, so a resize only redraws on a change. */
  private drawnFor = 0;
  /** The commit the list was last scrolled to, so it only moves on a change. */
  private scrolledTo: string | null = null;

  static get observedAttributes(): string[] {
    return ['minimized'];
  }

  attributeChangedCallback(name: string): void {
    if (name === 'minimized') this.syncMinimizeButton();
  }

  connectedCallback(): void {
    // Adding the same handler twice is a no-op, so this also restores what
    // leaving the document gave up.
    window.addEventListener('pointerup', this.endScrub);
    // The panel is hidden until an index loads, and a scrollbar appearing
    // narrows the rows; both change how much room the lanes have.
    this.resizes ??= new ResizeObserver(() => this.drawGraph());
    if (this.shadowRoot) {
      this.resizes.observe(this.viewport);
      return;
    }

    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = TEMPLATE;
    this.viewport = root.querySelector('.scroll') as HTMLElement;
    this.body = root.querySelector('.body') as HTMLElement;
    this.count = root.querySelector('.count') as HTMLElement;

    this.minimizeButton = root.querySelector('.minimize') as HTMLButtonElement;
    this.minimizeButton.addEventListener('click', () => this.emitPanelAction('timeline-minimize'));
    root.querySelector('.close')?.addEventListener('click', () => this.emitPanelAction('timeline-close'));

    this.body.addEventListener('pointerdown', this.onPointerDown);
    this.body.addEventListener('pointermove', this.onPointerMove);
    this.viewport.addEventListener('pointerleave', this.onPointerLeave);
    this.resizes.observe(this.viewport);

    this.syncMinimizeButton();
    this.render();
  }

  disconnectedCallback(): void {
    // A pointer released outside the graph must also end a scrub, which is
    // why the listener is on the window and has to be taken back here.
    window.removeEventListener('pointerup', this.endScrub);
    this.resizes?.disconnect();
  }

  setData(snapshots: Snapshot[], selected: string[]): void {
    this.snapshots = snapshots;
    this.graph = buildCommitGraph(snapshots);
    this.selected = new Set(selected);
    this.keepAnchor(selected);
    this.scrolledTo = null;
    this.render();
  }

  setSelection(selected: string[]): void {
    this.selected = new Set(selected);
    this.keepAnchor(selected);
    // Only the row styling changes, so the graph itself is left standing.
    this.paintSelection();
    this.reveal();
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

  private emitPanelAction(name: 'timeline-minimize' | 'timeline-close'): void {
    this.dispatchEvent(new CustomEvent(name, { bubbles: true, composed: true }));
  }

  private syncMinimizeButton(): void {
    if (!this.minimizeButton) return;
    const minimized = this.hasAttribute('minimized');
    this.minimizeButton.textContent = minimized ? '□' : '−';
    this.minimizeButton.setAttribute('aria-label', minimized ? 'Restore timeline' : 'Minimize timeline');
    this.minimizeButton.title = minimized ? 'Restore timeline' : 'Minimize timeline';
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

  // ------------------------------------------------------------- pointing --

  /** The row an event landed on, or -1 for the padding around them. */
  private rowAt(event: PointerEvent): number {
    const row = (event.target as HTMLElement | null)?.closest?.('.row') as HTMLElement | null;
    const index = row?.dataset.row;
    return index === undefined ? -1 : Number(index);
  }

  private onPointerDown = (event: PointerEvent): void => {
    const index = this.rowAt(event);
    if (index < 0) return;
    this.scrubbing = !event.shiftKey && !event.ctrlKey && !event.metaKey;
    this.select(index, event);
  };

  private onPointerMove = (event: PointerEvent): void => {
    const index = this.rowAt(event);
    if (index < 0) {
      this.emitHover(null, 0, 0);
      return;
    }
    const snapshot = this.graph.nodes[index].snapshot;
    this.emitHover(snapshot, event.clientX, event.clientY);
    // Dragging across the graph walks the selection along with the pointer,
    // narrowing a range back down to the one commit under it.
    if (!this.scrubbing) return;
    const alone = this.selected.size === 1 && this.selected.has(snapshot.oid);
    if (!alone) this.emitSelection([snapshot.oid]);
  };

  private onPointerLeave = (): void => {
    this.scrubbing = false;
    this.emitHover(null, 0, 0);
  };

  private endScrub = (): void => {
    this.scrubbing = false;
  };

  private select(index: number, event: PointerEvent | MouseEvent): void {
    const node = this.graph.nodes[index];
    if (!node) return;
    const oid = node.snapshot.oid;
    const order = this.graph.nodes.map((entry) => entry.snapshot.oid);

    if (event.shiftKey && this.anchor) {
      const from = order.indexOf(this.anchor);
      if (from !== -1) {
        const [low, high] = from <= index ? [from, index] : [index, from];
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

  // ------------------------------------------------------------- drawing --

  private render(): void {
    if (!this.body) return;
    this.count.textContent = this.snapshots.length > 0 ? `${this.snapshots.length} commits` : '';
    this.body.textContent = '';
    this.rows = [];
    this.lines = null;
    this.drawnFor = 0;

    if (this.graph.nodes.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty';
      empty.textContent = 'No commits loaded.';
      this.body.append(empty);
      return;
    }

    for (const node of this.graph.nodes) this.rows.push(buildRow(node.snapshot, node.row));
    this.body.append(...this.rows);

    // Added last so the lines and nodes paint over the row backgrounds; a
    // selected row must not swallow the branch crossing it.
    this.lines = document.createElementNS(SVG_NS, 'svg');
    this.lines.setAttribute('class', 'lines');
    this.body.append(this.lines);

    this.paintSelection();
    this.drawGraph();
    this.reveal();
  }

  private paintSelection(): void {
    this.rows.forEach((row, index) => {
      row.classList.toggle('selected', this.selected.has(this.graph.nodes[index].snapshot.oid));
    });
  }

  /**
   * Place the lanes and draw them. Lanes keep their spacing until the graph
   * would leave the message no room, and are squeezed together after that.
   */
  private drawGraph(): void {
    if (!this.lines) return;
    const width = this.viewport.clientWidth;
    // Hidden, or not laid out yet: there is nothing to measure against.
    if (width <= 0 || width === this.drawnFor) return;
    this.drawnFor = width;

    const spread = this.graph.lanes - 1;
    const room = Math.max(0, width - TEXT_MIN - GRAPH_PAD * 2);
    const step = spread === 0 ? LANE : Math.max(LANE_MIN, Math.min(LANE, room / spread));
    const graph = GRAPH_PAD * 2 + spread * step;
    const height = this.graph.nodes.length * ROW;

    this.body.style.setProperty('--graph', `${graph}px`);
    // A graph too wide even for the squeeze scrolls sideways rather than
    // sitting on top of the messages.
    this.body.style.minWidth = `${graph + TEXT_MIN}px`;

    this.lines.setAttribute('width', String(graph));
    this.lines.setAttribute('height', String(height));
    this.lines.setAttribute('viewBox', `0 0 ${graph} ${height}`);
    this.lines.textContent = '';

    const laneX = (lane: number) => GRAPH_PAD + lane * step;
    const rowY = (row: number) => row * ROW + ROW / 2;

    const fragment = document.createDocumentFragment();
    for (const edge of this.graph.edges) {
      const path = edgePath(
        laneX(edge.fromLane),
        rowY(edge.fromRow),
        laneX(edge.toLane),
        rowY(edge.toRow),
        edge.runsIn === edge.toLane,
      );
      fragment.append(line(path));
    }
    for (const node of this.graph.nodes) {
      if (!node.truncated) continue;
      const x = laneX(node.lane);
      const y = rowY(node.row);
      fragment.append(line(`M ${x} ${y} V ${y + STUB}`, true));
    }
    for (const node of this.graph.nodes) {
      const dot = document.createElementNS(SVG_NS, 'circle');
      dot.setAttribute('cx', String(laneX(node.lane)));
      dot.setAttribute('cy', String(rowY(node.row)));
      dot.setAttribute('r', '4.5');
      // A merge is drawn hollow, so a commit that brings two lines together is
      // recognisable without reading its message.
      dot.setAttribute('fill', node.merge ? 'var(--surface)' : 'var(--text-muted)');
      dot.setAttribute('stroke', 'var(--text-muted)');
      dot.setAttribute('stroke-width', '1.75');
      fragment.append(dot);
    }
    this.lines.append(fragment);
  }

  /**
   * Bring the first selected commit into view. With a long history a scrub or
   * a range selection easily lands off screen.
   */
  private reveal(): void {
    const first = this.graph.nodes.findIndex((node) => this.selected.has(node.snapshot.oid));
    if (first < 0) return;
    const oid = this.graph.nodes[first].snapshot.oid;
    // Only move when the selection changed, or the list would fight the user
    // scrolling through it.
    if (oid === this.scrolledTo) return;
    this.scrolledTo = oid;
    this.rows[first]?.scrollIntoView({ block: 'nearest' });
  }
}

function line(path: string, dashed = false): SVGPathElement {
  const element = document.createElementNS(SVG_NS, 'path');
  element.setAttribute('d', path);
  element.setAttribute('fill', 'none');
  element.setAttribute('stroke', 'var(--text-faint)');
  element.setAttribute('stroke-width', '1.75');
  element.setAttribute('stroke-linecap', 'round');
  if (dashed) element.setAttribute('stroke-dasharray', '2 3');
  return element;
}

/**
 * A line from a commit to one of its parents.
 *
 * `intoParent` says the straight run belongs to the parent's lane, so the
 * corner is turned at the top, right under the commit: that is a merge
 * reaching out to the branch it took in. Otherwise the run stays in the
 * commit's own lane and turns in at the bottom, which is a branch rejoining
 * the line it grew out of.
 */
function edgePath(
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  intoParent: boolean,
): string {
  if (x1 === x2) return `M ${x1} ${y1} V ${y2}`;
  const down = y2 >= y1 ? 1 : -1;
  const step = Math.sign(x2 - x1);
  const radius = Math.max(2, Math.min(CORNER, Math.abs(x2 - x1), Math.abs(y2 - y1) / 2));
  if (intoParent) {
    const turn = y1 + down * radius;
    return `M ${x1} ${y1} H ${x2 - step * radius} Q ${x2} ${y1} ${x2} ${turn} V ${y2}`;
  }
  const turn = y2 - down * radius;
  return `M ${x1} ${y1} V ${turn} Q ${x1} ${y2} ${x1 + step * radius} ${y2} H ${x2}`;
}

function buildRow(snapshot: Snapshot, index: number): HTMLElement {
  const row = document.createElement('div');
  row.className = 'row';
  row.dataset.row = String(index);

  if (snapshot.refs.length > 0) row.append(...buildRefs(snapshot.refs));

  const message = document.createElement('span');
  message.className = snapshot.summary ? 'message' : 'message empty-message';
  message.textContent = snapshot.summary || '(no message)';
  row.append(message);

  const oid = document.createElement('span');
  oid.className = 'oid';
  oid.textContent = snapshot.oid.slice(0, 7);
  row.append(oid);
  return row;
}

/** Branch and tag chips, capped so a busy commit cannot crowd out its message. */
function buildRefs(refs: string[]): HTMLElement[] {
  const strip = document.createElement('span');
  strip.className = 'refs';
  for (const ref of refs.slice(0, REF_LIMIT)) {
    const chip = document.createElement('span');
    // The runner shortens every ref, which leaves a remote as `origin/main`.
    chip.className = ref.includes('/') ? 'chip remote' : 'chip';
    chip.textContent = ref;
    strip.append(chip);
  }
  if (refs.length <= REF_LIMIT) return [strip];

  // The counter stands beside the strip rather than inside it: the names are
  // free to be trimmed, but how many were left out is not.
  const more = document.createElement('span');
  more.className = 'chip more';
  more.textContent = `+${refs.length - REF_LIMIT}`;
  more.title = refs.slice(REF_LIMIT).join(', ');
  return [strip, more];
}

customElements.define('cellular-timeline', CellularTimeline);
