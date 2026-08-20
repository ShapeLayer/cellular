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
    .editor { display: flex; flex: 1 1 auto; overflow: auto; padding: 5px 0; font-family: var(--mono); font-size: 12px; }
    .gutter { flex: none; width: 30px; border-right: 1px solid var(--border); }
    .gutter-row { height: 25px; }
    .toggle {
      appearance: none;
      display: block;
      width: 100%;
      height: 25px;
      border: 0;
      background: transparent;
      color: var(--danger);
      font: inherit;
      cursor: pointer;
    }
    .toggle.include { color: var(--accent); }
    .toggle:hover { background: var(--surface-hover); }
    textarea {
      min-width: 0;
      flex: 1;
      box-sizing: border-box;
      border: 0;
      outline: 0;
      min-height: 25px;
      padding: 0 10px;
      background: transparent;
      color: var(--text);
      font: inherit;
      line-height: 25px;
      white-space: pre;
      overflow: hidden;
      resize: none;
    }
    .editor:focus-within { background: var(--surface-hover); }
  </style>
  <header><span>Filters</span><button class="close" type="button" aria-label="Close filters" title="Close filters">×</button></header>
  <div class="help">− excludes matching paths or names · + includes them again.</div>
  <div class="editor"><div class="gutter"></div><textarea aria-label="Filter rules" wrap="off" spellcheck="false"></textarea></div>
`;

export class CellularFilters extends HTMLElement {
  private rules: FilterRule[] = [{ include: false, pattern: '' }];
  private gutter!: HTMLElement;
  private editor!: HTMLTextAreaElement;

  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = TEMPLATE;
    this.gutter = root.querySelector('.gutter') as HTMLElement;
    this.editor = root.querySelector('textarea') as HTMLTextAreaElement;
    root.querySelector('.close')?.addEventListener('click', () => {
      this.dispatchEvent(new CustomEvent('filters-close', { bubbles: true, composed: true }));
    });
    this.editor.addEventListener('input', this.onInput);
    this.renderEditor();
  }

  setRules(rules: FilterRule[]): void {
    this.rules = rules.length > 0 ? rules.map((rule) => ({ ...rule })) : [{ include: false, pattern: '' }];
    this.renderEditor();
  }

  focusEditor(): void {
    requestAnimationFrame(() => this.editor.focus());
  }

  private renderEditor(): void {
    if (!this.editor) return;
    this.editor.value = this.rules.map((rule) => rule.pattern).join('\n');
    this.renderGutter();
    this.fitEditorHeight();
  }

  private renderGutter(): void {
    this.gutter.textContent = '';
    this.rules.forEach((rule, index) => {
      const row = document.createElement('div');
      row.className = 'gutter-row';
      const toggle = document.createElement('button');
      toggle.className = rule.include ? 'toggle include' : 'toggle';
      toggle.type = 'button';
      toggle.textContent = rule.include ? '+' : '−';
      toggle.title = rule.include ? 'Include matching paths' : 'Exclude matching paths';
      toggle.addEventListener('click', () => {
        this.rules[index].include = !this.rules[index].include;
        this.emitChange();
        this.renderGutter();
      });
      row.append(toggle);
      this.gutter.append(row);
    });
  }

  private onInput = (): void => {
    const lines = this.editor.value.split('\n');
    // Preserve a marker with its unchanged text when rows move because of a
    // multi-line edit. Newly typed lines always start as exclusions.
    const remaining = [...this.rules];
    this.rules = lines.map((pattern) => {
      const previous = remaining.findIndex((rule) => rule.pattern === pattern);
      const include = previous === -1 ? false : remaining.splice(previous, 1)[0].include;
      return { include, pattern };
    });
    this.renderGutter();
    this.fitEditorHeight();
    this.emitChange();
  };

  private fitEditorHeight(): void {
    this.editor.style.height = '0';
    this.editor.style.height = `${Math.max(25, this.editor.scrollHeight)}px`;
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
