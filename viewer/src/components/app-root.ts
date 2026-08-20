/**
 * The viewer shell: owns the loaded index, the selection and the settings, and
 * wires the canvas, the menu, the timeline and the tooltip together.
 */

import './canvas';
import './filters';
import './menu-bar';
import './timeline';
import './tooltip';

import { readCellExport, EXPORT_EXTENSION } from '../data/cellexport';
import {
  availableMetrics,
  formatCommitTime,
  languagesAvailable,
  shortOid,
  METRIC_LABELS,
  type Metric,
  type Snapshot,
} from '../model';
import { buildScene, type Scene } from '../render/scene';
import { loadSettings, saveSettings, type Settings } from '../state/store';
import type { CellularCanvas, HoverDetail } from './canvas';
import type { CellularFilters, FilterRule } from './filters';
import type { MenuActionDetail } from './menu-bar';
import type { CellularTimeline, CommitHoverDetail } from './timeline';
import type { CellularTooltip } from './tooltip';

const TEMPLATE = `
  <style>
    :host { position: relative; display: block; width: 100%; height: 100%; }
    cellular-canvas { z-index: 0; }

    .drop {
      position: fixed;
      inset: 0;
      z-index: 20;
      display: none;
      place-content: center;
      justify-items: center;
      gap: 12px;
      background: rgba(251, 251, 250, 0.86);
      backdrop-filter: blur(1px);
      font-family: var(--font);
      color: var(--text);
      pointer-events: none;
    }
    :host([dropping]) .drop { display: grid; }
    .drop .frame {
      border: 2px dashed var(--accent);
      border-radius: var(--radius-lg);
      padding: 34px 54px;
      text-align: center;
      background: var(--surface);
      box-shadow: var(--shadow-md);
    }
    .drop .frame strong { display: block; font-size: 16px; margin-bottom: 4px; }
    .drop .frame span { color: var(--text-muted); font-size: 13px; }

    .panel {
      position: fixed;
      left: 12px;
      bottom: 12px;
      z-index: 30;
      max-width: 340px;
      max-height: 46vh;
      overflow-y: auto;
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      box-shadow: var(--shadow-sm);
      font-family: var(--font);
      font-size: 12px;
      color: var(--text);
      overflow: hidden;
    }
    .panel:empty { display: none; }
    .panel section { padding: 8px 10px; }
    .panel section + section { border-top: 1px solid var(--border); }
    .legend { display: flex; flex-wrap: wrap; gap: 4px 12px; }
    .legend .entry { display: flex; align-items: center; gap: 6px; }
    .legend .swatch {
      width: 10px; height: 10px; border-radius: 2px;
      border: 1px solid rgba(31, 35, 40, 0.12);
      flex: none;
    }
    .note { color: var(--text-muted); }
    .problem { color: var(--danger); }
    .problem strong { display: block; }
    .dismiss {
      appearance: none; border: 0; background: transparent; color: var(--text-muted);
      font: inherit; cursor: pointer; padding: 0; margin-top: 4px; text-decoration: underline;
    }
    input[type='file'] { display: none; }
  </style>
  <cellular-canvas></cellular-canvas>
  <div class="drop"><div class="frame">
    <strong>Drop to open</strong>
    <span>Release a <code>.cellexport</code> file here to load its index.</span>
  </div></div>
  <cellular-menu></cellular-menu>
  <cellular-filters hidden></cellular-filters>
  <cellular-timeline hidden></cellular-timeline>
  <cellular-tooltip></cellular-tooltip>
  <div class="panel"></div>
  <input type="file" accept=".cellexport,application/octet-stream" />
`;

export class CellularApp extends HTMLElement {
  private settings: Settings = loadSettings();
  private snapshots: Snapshot[] = [];
  private selected: string[] = [];
  private problems: string[] = [];
  /** Explanations about the loaded index, shown under the legend. */
  private notes: string[] = [];
  private scene: Scene | null = null;

  private canvas!: CellularCanvas;
  private menu!: HTMLElement & {
    setState(settings: Settings, available?: Metric[]): void;
  };
  private timeline!: CellularTimeline;
  private filters!: CellularFilters;
  private tooltip!: CellularTooltip;
  private panel!: HTMLElement;
  private picker!: HTMLInputElement;
  private dragDepth = 0;
  private filterRules: FilterRule[] = [];

  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = TEMPLATE;

    this.canvas = root.querySelector('cellular-canvas') as CellularCanvas;
    this.menu = root.querySelector('cellular-menu') as HTMLElement & {
      setState(settings: Settings, available?: Metric[]): void;
    };
    this.timeline = root.querySelector('cellular-timeline') as CellularTimeline;
    this.filters = root.querySelector('cellular-filters') as CellularFilters;
    this.tooltip = root.querySelector('cellular-tooltip') as CellularTooltip;
    this.panel = root.querySelector('.panel') as HTMLElement;
    this.picker = root.querySelector('input[type=file]') as HTMLInputElement;

    root.addEventListener('menu-action', (event) =>
      this.onMenuAction((event as CustomEvent<MenuActionDetail>).detail),
    );
    root.addEventListener('cell-hover', (event) =>
      this.onCellHover((event as CustomEvent<HoverDetail>).detail),
    );
    root.addEventListener('commit-hover', (event) =>
      this.onCommitHover((event as CustomEvent<CommitHoverDetail>).detail),
    );
    root.addEventListener('commit-select', (event) =>
      this.setSelection((event as CustomEvent<{ oids: string[] }>).detail.oids),
    );
    root.addEventListener('timeline-minimize', () => this.toggleTimelineMinimized());
    root.addEventListener('timeline-close', () => this.closeTimeline());
    root.addEventListener('filters-change', (event) => {
      this.filterRules = (event as CustomEvent<FilterRule[]>).detail;
      this.rebuild(true);
    });
    root.addEventListener('filters-close', () => this.filters.setAttribute('hidden', ''));
    this.picker.addEventListener('change', () => {
      const file = this.picker.files?.[0];
      if (file) void this.openFile(file);
      this.picker.value = '';
    });

    window.addEventListener('dragenter', this.onDragEnter);
    window.addEventListener('dragover', this.onDragOver);
    window.addEventListener('dragleave', this.onDragLeave);
    window.addEventListener('drop', this.onDrop);
    window.addEventListener('keydown', this.onKeyDown);

    this.menu.setState(this.settings);
    this.applyTimelineVisibility();
    this.renderPanel();
    void this.openFromQuery();
  }

  disconnectedCallback(): void {
    window.removeEventListener('dragenter', this.onDragEnter);
    window.removeEventListener('dragover', this.onDragOver);
    window.removeEventListener('dragleave', this.onDragLeave);
    window.removeEventListener('drop', this.onDrop);
    window.removeEventListener('keydown', this.onKeyDown);
  }

  // -------------------------------------------------------------- loading --

  /**
   * Load an index from raw bytes. Public so a page can hand the viewer an
   * index it fetched itself, which is also how `?src=` works.
   */
  load(bytes: Uint8Array): void {
    this.tooltip.hide();
    try {
      const loaded = readCellExport(bytes);
      this.snapshots = loaded.snapshots;
      this.problems = loaded.warnings;
      this.notes = [];
      const newest = loaded.snapshots[loaded.snapshots.length - 1];
      this.selected = newest ? [newest.oid] : [];
      this.applyAvailableMetrics();
      this.timeline.setData(this.snapshots, this.selected);
      this.rebuild(false);
    } catch (error) {
      this.problems = [(error as Error).message];
      this.renderPanel();
    }
  }

  private async openFile(file: File): Promise<void> {
    this.load(new Uint8Array(await file.arrayBuffer()));
  }

  /** Open an index named by the `src` query parameter, for shareable links. */
  private async openFromQuery(): Promise<void> {
    const source = new URLSearchParams(location.search).get('src');
    if (!source) return;
    try {
      const response = await fetch(source);
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      this.load(new Uint8Array(await response.arrayBuffer()));
    } catch (error) {
      this.problems = [`could not fetch ${source}: ${(error as Error).message}`];
      this.renderPanel();
    }
  }

  private onDragEnter = (event: DragEvent): void => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    this.dragDepth += 1;
    this.setAttribute('dropping', '');
  };

  private onDragOver = (event: DragEvent): void => {
    if (!hasFiles(event)) return;
    // Without this the browser navigates to the dropped file instead.
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
  };

  private onDragLeave = (event: DragEvent): void => {
    if (!hasFiles(event)) return;
    this.dragDepth = Math.max(0, this.dragDepth - 1);
    if (this.dragDepth === 0) this.removeAttribute('dropping');
  };

  private onDrop = (event: DragEvent): void => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    this.dragDepth = 0;
    this.removeAttribute('dropping');
    const file = event.dataTransfer?.files?.[0];
    if (file) void this.openFile(file);
  };

  // ------------------------------------------------------------- commands --

  private onMenuAction(detail: MenuActionDetail): void {
    switch (detail.action) {
      case 'open':
        this.picker.click();
        break;
      case 'open-filters':
        this.filters.removeAttribute('hidden');
        this.filters.setRules(this.filterRules);
        this.filters.focusEditor();
        break;
      case 'zoom-in':
        this.canvas.zoomBy(1.25);
        break;
      case 'zoom-out':
        this.canvas.zoomBy(1 / 1.25);
        break;
      case 'reset-zoom':
        this.canvas.resetZoom();
        break;
      case 'fit':
        this.canvas.fit();
        break;
      case 'toggle-timeline':
        this.update({
          timelineVisible: !this.settings.timelineVisible,
          // Opening from Window always restores the full panel.
          timelineMinimized: this.settings.timelineVisible ? this.settings.timelineMinimized : false,
        });
        this.applyTimelineVisibility();
        break;
      case 'toggle-legends':
        this.update({ legendsVisible: !this.settings.legendsVisible });
        this.renderPanel();
        this.applySafeArea();
        this.canvas.fit();
        break;
      case 'multi-mode':
        this.update({ multiCommitMode: detail.value as Settings['multiCommitMode'] });
        this.rebuild(true);
        break;
      case 'multi-commit-size-reference':
        this.update({ multiCommitSizeReference: detail.value as Settings['multiCommitSizeReference'] });
        this.rebuild(true);
        break;
      case 'overlay-style':
        this.update({ overlayStyle: detail.value as Settings['overlayStyle'] });
        this.rebuild(true);
        break;
      case 'metric':
        this.update({ metric: detail.value as Settings['metric'] });
        this.rebuild(true);
        break;
      default:
        break;
    }
  }

  private onKeyDown = (event: KeyboardEvent): void => {
    if (!(event.ctrlKey || event.metaKey)) return;
    if (event.key === '+' || event.key === '=') {
      event.preventDefault();
      this.canvas.zoomBy(1.25);
    } else if (event.key === '-') {
      event.preventDefault();
      this.canvas.zoomBy(1 / 1.25);
    } else if (event.key === '0') {
      event.preventDefault();
      this.canvas.fit();
    }
  };

  private update(patch: Partial<Settings>): void {
    this.settings = { ...this.settings, ...patch };
    saveSettings(this.settings);
    this.menu.setState(this.settings);
  }

  /**
   * Keep the chosen metric to something this index actually holds, and say so
   * rather than drawing an empty screen.
   */
  private applyAvailableMetrics(): void {
    const available = availableMetrics(this.snapshots);
    if (!available.includes(this.settings.metric)) {
      const fallback = available[0] as Metric;
      this.notes.push(
        `This index was built without ${METRIC_LABELS[this.settings.metric].toLowerCase()}; showing ${METRIC_LABELS[
          fallback
        ].toLowerCase()} instead.`,
      );
      this.update({ metric: fallback });
    }
    if (!languagesAvailable(this.snapshots)) {
      this.notes.push('This index has no language breakdown, so blocks are left uncoloured.');
    }
    this.menu.setState(this.settings, available);
  }

  private applyTimelineVisibility(): void {
    this.timeline.toggleAttribute('hidden', !this.settings.timelineVisible);
    this.timeline.toggleAttribute('minimized', this.settings.timelineVisible && this.settings.timelineMinimized);
    this.applySafeArea();
  }

  private toggleTimelineMinimized(): void {
    if (!this.settings.timelineVisible) return;
    this.update({ timelineMinimized: !this.settings.timelineMinimized });
    this.applyTimelineVisibility();
  }

  private closeTimeline(): void {
    this.update({ timelineVisible: false, timelineMinimized: false });
    this.applyTimelineVisibility();
  }

  /** Keep the fitted scene clear of the menu, the timeline and the legend. */
  private applySafeArea(): void {
    const panelHeight = this.panel.getBoundingClientRect().height;
    this.canvas.setSafeArea({
      top: 56,
      right: this.settings.timelineVisible && !this.settings.timelineMinimized ? 320 : 24,
      bottom: Math.max(24, panelHeight + 24),
      left: 24,
    });
  }

  private setSelection(oids: string[]): void {
    if (oids.length === 0) return;
    const same =
      oids.length === this.selected.length && oids.every((oid, index) => oid === this.selected[index]);
    if (same) return;
    this.selected = oids;
    this.timeline.setSelection(oids);
    this.rebuild(true);
  }

  private rebuild(keepViewportIfPossible: boolean): void {
    const chosen = filterSnapshots(
      this.snapshots.filter((snapshot) => this.selected.includes(snapshot.oid)),
      this.filterRules,
    );
    const previous = this.scene;
    this.scene = buildScene(chosen, this.settings);
    const sameShape =
      keepViewportIfPossible &&
      previous !== null &&
      this.scene !== null &&
      previous.bounds.width === this.scene.bounds.width &&
      previous.bounds.height === this.scene.bounds.height;
    this.renderPanel();
    this.applySafeArea();
    this.canvas.setScene(this.scene, sameShape);
  }

  // ------------------------------------------------------------- tooltips --

  private onCellHover(detail: HoverDetail): void {
    if (!detail.cell) {
      this.tooltip.hide();
      return;
    }
    this.tooltip.show(detail.cell.tooltip, detail.clientX, detail.clientY);
  }

  private onCommitHover(detail: CommitHoverDetail): void {
    const snapshot = detail.snapshot;
    if (!snapshot) {
      this.tooltip.hide();
      return;
    }
    this.tooltip.show(
      {
        title: shortOid(snapshot.oid),
        subtitle: snapshot.summary || '(no message)',
        rows: [
          ['Author', snapshot.author || 'unknown'],
          ['Date', formatCommitTime(snapshot)],
          ...(snapshot.refs.length > 0
            ? ([['Refs', snapshot.refs.join(', ')]] as Array<[string, string]>)
            : []),
          ['Modules', String(snapshot.modules.length)],
        ],
      },
      detail.clientX,
      detail.clientY,
    );
  }

  // ---------------------------------------------------------------- panel --

  private renderPanel(): void {
    this.panel.textContent = '';

    if (this.problems.length > 0) {
      const section = document.createElement('section');
      section.className = 'problem';
      const heading = document.createElement('strong');
      heading.textContent = this.problems.length === 1 ? 'Could not load' : 'Loaded with warnings';
      section.append(heading);
      for (const problem of this.problems) {
        const line = document.createElement('div');
        line.textContent = problem;
        section.append(line);
      }
      const dismiss = document.createElement('button');
      dismiss.className = 'dismiss';
      dismiss.type = 'button';
      dismiss.textContent = 'Dismiss';
      dismiss.addEventListener('click', () => {
        this.problems = [];
        this.renderPanel();
      });
      section.append(dismiss);
      this.panel.append(section);
    }

    if (!this.scene) return;

    if (this.settings.legendsVisible && this.scene.legend.length > 0) {
      const section = document.createElement('section');
      section.className = 'legend';
      for (const entry of this.scene.legend) {
        const holder = document.createElement('span');
        holder.className = 'entry';
        const swatch = document.createElement('span');
        swatch.className = 'swatch';
        swatch.style.background = entry.colour;
        const label = document.createElement('span');
        label.textContent = entry.label;
        holder.append(swatch, label);
        section.append(holder);
      }
      this.panel.append(section);
    }

    if (this.scene.note) {
      const section = document.createElement('section');
      section.className = 'note';
      section.textContent = this.scene.note;
      this.panel.append(section);
    }

    for (const note of this.notes) {
      const section = document.createElement('section');
      section.className = 'note';
      section.textContent = note;
      this.panel.append(section);
    }
  }
}

/** An inclusion wins over exclusion, so a specific `+` can restore one child of a `-` path. */
function filterSnapshots(snapshots: Snapshot[], rules: FilterRule[]): Snapshot[] {
  const active = rules.filter((rule) => rule.pattern.trim() !== '');
  if (active.length === 0) return snapshots;
  return snapshots.map((snapshot) => ({
    ...snapshot,
    modules: snapshot.modules.filter((module) => includeModule(module.path, active)),
  }));
}

function includeModule(path: string, rules: FilterRule[]): boolean {
  const matches = (rule: FilterRule) => {
    const pattern = rule.pattern.trim().replace(/^\.\//, '');
    const name = path.slice(path.lastIndexOf('/') + 1);
    return pattern.endsWith('/') ? path.startsWith(pattern) : path === pattern || name === pattern;
  };
  // Any explicit include takes priority over all matching exclusions.
  if (rules.some((rule) => rule.include && matches(rule))) return true;
  return !rules.some((rule) => !rule.include && matches(rule));
}

function hasFiles(event: DragEvent): boolean {
  return Array.from(event.dataTransfer?.types ?? []).includes('Files');
}

export { EXPORT_EXTENSION };

customElements.define('cellular-app', CellularApp);
