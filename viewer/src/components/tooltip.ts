/** A small floating panel that follows the pointer. */

import type { TooltipContent } from '../render/scene';

const TEMPLATE = `
  <style>
    :host {
      position: fixed;
      z-index: 40;
      pointer-events: none;
      display: none;
      max-width: 320px;
    }
    :host([open]) { display: block; }
    .card {
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      box-shadow: var(--shadow-md);
      padding: 8px 10px;
      font-family: var(--font);
      font-size: 12px;
      color: var(--text);
    }
    .title {
      font-family: var(--mono);
      font-size: 12px;
      font-weight: 600;
      word-break: break-all;
    }
    .subtitle {
      color: var(--text-muted);
      margin-top: 2px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    table { border-collapse: collapse; margin-top: 6px; width: 100%; }
    td { padding: 1px 0; vertical-align: top; }
    td.key { color: var(--text-muted); padding-right: 14px; white-space: nowrap; }
    td.value { text-align: right; font-family: var(--mono); white-space: nowrap; }
    tr.rule td { border-top: 1px solid var(--border); padding-top: 4px; font-size: 0; height: 4px; }
  </style>
  <div class="card">
    <div class="title"></div>
    <div class="subtitle"></div>
    <table></table>
  </div>
`;

export class CellularTooltip extends HTMLElement {
  private titleEl!: HTMLElement;
  private subtitleEl!: HTMLElement;
  private tableEl!: HTMLTableElement;

  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = TEMPLATE;
    this.titleEl = root.querySelector('.title') as HTMLElement;
    this.subtitleEl = root.querySelector('.subtitle') as HTMLElement;
    this.tableEl = root.querySelector('table') as HTMLTableElement;
  }

  show(content: TooltipContent, clientX: number, clientY: number): void {
    this.titleEl.textContent = content.title;
    this.subtitleEl.textContent = content.subtitle ?? '';
    this.subtitleEl.style.display = content.subtitle ? '' : 'none';

    this.tableEl.textContent = '';
    for (const [key, value] of content.rows) {
      const row = document.createElement('tr');
      if (key === '—') {
        row.className = 'rule';
        row.innerHTML = '<td colspan="2"></td>';
      } else {
        const keyCell = document.createElement('td');
        keyCell.className = 'key';
        keyCell.textContent = key;
        const valueCell = document.createElement('td');
        valueCell.className = 'value';
        valueCell.textContent = value;
        row.append(keyCell, valueCell);
      }
      this.tableEl.append(row);
    }

    this.setAttribute('open', '');
    this.place(clientX, clientY);
  }

  hide(): void {
    this.removeAttribute('open');
  }

  /** Keep the card inside the window, flipping sides when it would overflow. */
  private place(clientX: number, clientY: number): void {
    const offset = 14;
    const rect = this.getBoundingClientRect();
    let left = clientX + offset;
    let top = clientY + offset;
    if (left + rect.width > window.innerWidth - 8) left = clientX - offset - rect.width;
    if (top + rect.height > window.innerHeight - 8) top = clientY - offset - rect.height;
    this.style.left = `${Math.max(8, left)}px`;
    this.style.top = `${Math.max(8, top)}px`;
  }
}

customElements.define('cellular-tooltip', CellularTooltip);
