/** A compact, editor-like panel for including and excluding module paths. */

export interface FilterRule {
  include: boolean;
  pattern: string;
}

const TEMPLATE = `
  <style>
    :host {
      position: fixed;
      top: 56px;
      left: 12px;
      z-index: 31;
      width: min(520px, calc(100vw - 24px));
      max-height: min(62vh, 560px);
      display: flex;
      flex-direction: column;
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      box-shadow: var(--shadow-md);
      overflow: hidden;
      font-family: var(--font);
      color: var(--text);
    }
    :host([hidden]) { display: none; }
    header {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 10px;
      border-bottom: 1px solid var(--border);
      font-size: 12px;
      font-weight: 600;
    }
    header span { flex: 1; }
    .close {
      appearance: none;
      display: grid;
      place-items: center;
      width: 22px;
      height: 22px;
      margin: -4px -2px -4px 0;
      padding: 0;
      border: 0;
      border-radius: 4px;
      background: transparent;
      color: var(--text-muted);
      font: inherit;
      font-size: 16px;
      cursor: pointer;
    }
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
    .help { padding: 7px 10px; border-bottom: 1px solid var(--border); color: var(--text-muted); font-size: 11px; }
    .rows { overflow: auto; padding: 5px 0; font-family: var(--mono); font-size: 12px; }
    .row { display: flex; align-items: center; min-height: 25px; }
    .toggle {
      appearance: none;
      width: 30px;
      align-self: stretch;
      border: 0;
      border-right: 1px solid var(--border);
      background: transparent;
      color: var(--danger);
      font: inherit;
      cursor: pointer;
    }
    .toggle.include { color: var(--accent); }
    .toggle:hover { background: var(--surface-hover); }
    input {
      min-width: 0;
      flex: 1;
      box-sizing: border-box;
      border: 0;
      outline: 0;
      padding: 4px 10px;
      background: transparent;
      color: var(--text);
      font: inherit;
    }
    .row:focus-within { background: var(--surface-hover); }
  </style>
  <header><span>Filters</span><button class="close" type="button" aria-label="Close filters" title="Close filters">×</button></header>
  <div class="help">− excludes matching paths or names · + includes them again. Press Enter for a new rule.</div>
  <div class="rows"></div>
`;

export class CellularFilters extends HTMLElement {
  private rules: FilterRule[] = [{ include: false, pattern: '' }];
  private rows!: HTMLElement;

  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = TEMPLATE;
    this.rows = root.querySelector('.rows') as HTMLElement;
    root.querySelector('.close')?.addEventListener('click', () => {
      this.dispatchEvent(new CustomEvent('filters-close', { bubbles: true, composed: true }));
    });
    this.render();
  }

  setRules(rules: FilterRule[]): void {
    this.rules = rules.length > 0 ? rules.map((rule) => ({ ...rule })) : [{ include: false, pattern: '' }];
    this.render();
  }

  focusEditor(): void {
    requestAnimationFrame(() => (this.rows.querySelector('input') as HTMLInputElement | null)?.focus());
  }

  private render(): void {
    if (!this.rows) return;
    this.rows.textContent = '';
    this.rules.forEach((rule, index) => {
      const row = document.createElement('div');
      row.className = 'row';
      const toggle = document.createElement('button');
      toggle.className = rule.include ? 'toggle include' : 'toggle';
      toggle.type = 'button';
      toggle.textContent = rule.include ? '+' : '−';
      toggle.title = rule.include ? 'Include matching paths' : 'Exclude matching paths';
      toggle.addEventListener('click', () => {
        this.rules[index].include = !this.rules[index].include;
        this.emitChange();
        this.render();
      });
      const input = document.createElement('input');
      input.type = 'text';
      input.value = rule.pattern;
      input.placeholder = 'path or name';
      input.spellcheck = false;
      input.addEventListener('input', () => {
        this.rules[index].pattern = input.value;
        this.emitChange();
      });
      input.addEventListener('keydown', (event) => this.onKeyDown(event, index));
      row.append(toggle, input);
      this.rows.append(row);
    });
  }

  private onKeyDown(event: KeyboardEvent, index: number): void {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    this.rules.splice(index + 1, 0, { include: false, pattern: '' });
    this.emitChange();
    this.render();
    (this.rows.querySelectorAll('input')[index + 1] as HTMLInputElement | undefined)?.focus();
  }

  private emitChange(): void {
    this.dispatchEvent(
      new CustomEvent<FilterRule[]>('filters-change', {
        detail: this.rules.map((rule) => ({ ...rule })),
        bubbles: true,
        composed: true,
      }),
    );
  }
}

customElements.define('cellular-filters', CellularFilters);
