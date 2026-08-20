/**
 * The menu control, pinned to the top left of the window.
 *
 * Emits `menu-action` with `{ action, value }`; the app root decides what each
 * one does and feeds the current settings back through `setState`.
 */

import { METRIC_LABELS, type Metric } from '../model';
import {
  DEFAULT_SETTINGS,
  type MultiCommitMode,
  type MultiCommitSizeReference,
  type OverlayStyle,
  type Settings,
} from '../state/store';

export interface MenuActionDetail {
  action: string;
  value?: string;
}

interface Item {
  label: string;
  action?: string;
  value?: string;
  divider?: boolean;
  submenu?: Item[];
  checkedWhen?: (settings: Settings) => boolean;
  /** Metric this entry needs; greyed out when the index does not hold it. */
  needsMetric?: Metric;
}

const MULTI_LABELS: Record<MultiCommitMode, string> = {
  'side-by-side': 'Side by Side',
  overlay: 'Overlay',
};

const MULTI_COMMIT_SIZE_REFERENCE_LABELS: Record<MultiCommitSizeReference, string> = {
  newest: 'Use Newest Commit as Base Size',
  oldest: 'Use Oldest Commit as Base Size',
};

const OVERLAY_LABELS: Record<OverlayStyle, string> = {
  'change-highlight': 'Change Highlight',
  'translucent-layers': 'Translucent Layers',
  'split-cells': 'Split Cells',
};

const MENUS: Array<{ label: string; items: Item[] }> = [
  {
    label: 'File',
    items: [{ label: 'Open…', action: 'open' }],
  },
  {
    label: 'View',
    items: [
      { label: 'Zoom In', action: 'zoom-in' },
      { label: 'Zoom Out', action: 'zoom-out' },
      { label: 'Reset Zoom', action: 'reset-zoom' },
      { label: 'Fit to Screen', action: 'fit' },
      { label: '', divider: true },
      {
        label: 'Show Multiple Commits',
        submenu: [
          ...(Object.keys(MULTI_LABELS) as MultiCommitMode[]).map((mode) => ({
            label: MULTI_LABELS[mode],
            action: 'multi-mode',
            value: mode,
            checkedWhen: (settings: Settings) => settings.multiCommitMode === mode,
          })),
          { label: '', divider: true },
          ...(Object.keys(MULTI_COMMIT_SIZE_REFERENCE_LABELS) as MultiCommitSizeReference[]).map(
            (reference) => ({
              label: MULTI_COMMIT_SIZE_REFERENCE_LABELS[reference],
              action: 'multi-commit-size-reference',
              value: reference,
              checkedWhen: (settings: Settings) => settings.multiCommitSizeReference === reference,
            }),
          ),
        ],
      },
      {
        label: 'Overlay Style',
        submenu: (Object.keys(OVERLAY_LABELS) as OverlayStyle[]).map((style) => ({
          label: OVERLAY_LABELS[style],
          action: 'overlay-style',
          value: style,
          checkedWhen: (settings) => settings.overlayStyle === style,
        })),
      },
      {
        label: 'Metric',
        submenu: (Object.keys(METRIC_LABELS) as Metric[]).map((metric) => ({
          label: METRIC_LABELS[metric],
          action: 'metric',
          value: metric,
          needsMetric: metric,
          checkedWhen: (settings) => settings.metric === metric,
        })),
      },
    ],
  },
  {
    label: 'Panel',
    items: [
      {
        label: 'Timeline',
        action: 'toggle-timeline',
        checkedWhen: (settings) => settings.timelineVisible,
      },
      {
        label: 'Legends',
        action: 'toggle-legends',
        checkedWhen: (settings) => settings.legendsVisible,
      },
    ],
  },
];

const TEMPLATE = `
  <style>
    :host {
      position: fixed;
      top: 12px;
      left: 12px;
      z-index: 30;
      font-family: var(--font);
      font-size: 13px;
    }
    .bar {
      display: flex;
      align-items: center;
      gap: 2px;
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      box-shadow: var(--shadow-sm);
      padding: 3px;
    }
    .brand {
      font-weight: 600;
      color: var(--text);
      padding: 0 8px 0 6px;
      letter-spacing: 0.01em;
      border-right: 1px solid var(--border);
      margin-right: 3px;
      line-height: 22px;
    }
    button.top {
      appearance: none;
      border: 0;
      background: transparent;
      color: var(--text);
      font: inherit;
      padding: 4px 10px;
      border-radius: 4px;
      cursor: pointer;
    }
    button.top:hover { background: var(--surface-hover); }
    button.top[aria-expanded='true'] { background: var(--surface-active); }

    .menu {
      position: absolute;
      top: 100%;
      margin-top: 5px;
      min-width: 210px;
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      box-shadow: var(--shadow-md);
      padding: 4px;
      display: none;
    }
    .menu[open] { display: block; }

    .item {
      display: flex;
      align-items: center;
      gap: 8px;
      width: 100%;
      appearance: none;
      border: 0;
      background: transparent;
      color: var(--text);
      font: inherit;
      text-align: left;
      padding: 5px 8px;
      border-radius: 4px;
      cursor: pointer;
      position: relative;
    }
    .item:hover, .item.open { background: var(--surface-hover); }
    .item[disabled] { color: var(--text-faint); cursor: default; }
    .item[disabled]:hover { background: transparent; }
    .item .hint { color: var(--text-faint); font-size: 11px; }
    .item .check { width: 14px; color: var(--accent); flex: none; }
    .item .label { flex: 1; }
    .item .arrow { color: var(--text-faint); flex: none; }
    .divider { height: 1px; background: var(--border); margin: 4px 6px; }

    .submenu {
      position: absolute;
      left: calc(100% - 4px);
      top: -4px;
      min-width: 200px;
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      box-shadow: var(--shadow-md);
      padding: 4px;
      display: none;
    }
    .submenu[open] { display: block; }
    .holder { position: relative; }
  </style>
  <div class="bar">
    <span class="brand">Cellular</span>
  </div>
`;

export class CellularMenu extends HTMLElement {
  // The app root pushes the real settings straight after connecting; until
  // then the defaults keep the first render from reading undefined.
  private settings: Settings = DEFAULT_SETTINGS;
  /** Metrics the loaded index can answer for; empty until one is loaded. */
  private available: Metric[] = [];
  private bar!: HTMLElement;
  private openIndex: number | null = null;

  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = TEMPLATE;
    this.bar = root.querySelector('.bar') as HTMLElement;
    document.addEventListener('pointerdown', this.onDocumentPointerDown, true);
    document.addEventListener('keydown', this.onKeyDown);
    this.render();
  }

  disconnectedCallback(): void {
    document.removeEventListener('pointerdown', this.onDocumentPointerDown, true);
    document.removeEventListener('keydown', this.onKeyDown);
  }

  setState(settings: Settings, available: Metric[] = this.available): void {
    this.settings = settings;
    this.available = available;
    this.render();
  }

  private onDocumentPointerDown = (event: Event): void => {
    if (this.openIndex === null) return;
    if (event.composedPath().includes(this)) return;
    this.openIndex = null;
    this.render();
  };

  private onKeyDown = (event: KeyboardEvent): void => {
    if (event.key === 'Escape' && this.openIndex !== null) {
      this.openIndex = null;
      this.render();
    }
  };

  private emit(action: string, value?: string): void {
    this.openIndex = null;
    this.render();
    this.dispatchEvent(
      new CustomEvent<MenuActionDetail>('menu-action', {
        detail: { action, value },
        bubbles: true,
        composed: true,
      }),
    );
  }

  private render(): void {
    if (!this.bar) return;
    const settings = this.settings;
    this.bar.querySelectorAll('.holder').forEach((node) => node.remove());

    MENUS.forEach((menu, index) => {
      const holder = document.createElement('span');
      holder.className = 'holder';

      const button = document.createElement('button');
      button.className = 'top';
      button.type = 'button';
      button.textContent = menu.label;
      button.setAttribute('aria-expanded', String(this.openIndex === index));
      button.addEventListener('click', () => {
        this.openIndex = this.openIndex === index ? null : index;
        this.render();
      });
      holder.append(button);

      const panel = document.createElement('div');
      panel.className = 'menu';
      panel.setAttribute('role', 'menu');
      if (this.openIndex === index) panel.setAttribute('open', '');
      for (const item of menu.items) panel.append(this.renderItem(item, settings));
      holder.append(panel);

      this.bar.append(holder);
    });
  }

  private renderItem(item: Item, settings: Settings): HTMLElement {
    if (item.divider) {
      const divider = document.createElement('div');
      divider.className = 'divider';
      return divider;
    }

    if (item.submenu) {
      const holder = document.createElement('div');
      holder.className = 'holder';
      const button = this.itemButton(item, settings, '›');
      const panel = document.createElement('div');
      panel.className = 'submenu';
      for (const child of item.submenu) panel.append(this.renderItem(child, settings));

      const open = () => {
        panel.setAttribute('open', '');
        button.classList.add('open');
      };
      const close = () => {
        panel.removeAttribute('open');
        button.classList.remove('open');
      };
      holder.addEventListener('pointerenter', open);
      holder.addEventListener('pointerleave', close);
      button.addEventListener('click', (event) => {
        event.stopPropagation();
        if (panel.hasAttribute('open')) close();
        else open();
      });

      holder.append(button, panel);
      return holder;
    }

    const button = this.itemButton(item, settings);
    // With nothing loaded every metric stays available; once an index is in,
    // the ones it did not collect are shown but not offered.
    const unavailable =
      item.needsMetric !== undefined &&
      this.available.length > 0 &&
      !this.available.includes(item.needsMetric);
    if (unavailable) {
      button.disabled = true;
      button.setAttribute('disabled', '');
      const hint = document.createElement('span');
      hint.className = 'hint';
      hint.textContent = 'not collected';
      button.append(hint);
    } else {
      button.addEventListener('click', () => this.emit(item.action ?? '', item.value));
    }
    return button;
  }

  private itemButton(item: Item, settings: Settings, arrow?: string): HTMLButtonElement {
    const button = document.createElement('button');
    button.className = 'item';
    button.type = 'button';
    button.setAttribute('role', 'menuitem');

    const check = document.createElement('span');
    check.className = 'check';
    check.textContent = item.checkedWhen?.(settings) ? '✓' : '';

    const label = document.createElement('span');
    label.className = 'label';
    label.textContent = item.label;

    button.append(check, label);
    if (arrow) {
      const caret = document.createElement('span');
      caret.className = 'arrow';
      caret.textContent = arrow;
      button.append(caret);
    }
    return button;
  }
}

customElements.define('cellular-menu', CellularMenu);
