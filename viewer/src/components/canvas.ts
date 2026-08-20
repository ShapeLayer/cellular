/**
 * The heatmap canvas: draws the scene and handles pan, zoom and hover.
 *
 * The canvas fills the window. Layout happens once in world coordinates; this
 * component only applies a pan and zoom transform, so navigating never moves a
 * block relative to its neighbours.
 */

import type { Rect } from '../layout/squarify';
import type { Scene, SceneCell } from '../render/scene';

const MIN_SCALE = 0.02;
const MAX_SCALE = 24;
/** A block needs at least this much room on screen before it gets a label. */
const LABEL_MIN_WIDTH = 56;
const LABEL_MIN_HEIGHT = 26;

interface Viewport {
  x: number;
  y: number;
  scale: number;
}

/** Room the overlaid controls need, so fitting never hides the scene behind them. */
export interface SafeArea {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

export interface HoverDetail {
  cell: SceneCell | null;
  clientX: number;
  clientY: number;
}

const TEMPLATE = `
  <style>
    :host { position: absolute; inset: 0; display: block; }
    canvas { display: block; width: 100%; height: 100%; cursor: grab; }
    canvas.dragging { cursor: grabbing; }
    .empty {
      position: absolute;
      inset: 0;
      display: grid;
      place-content: center;
      justify-items: center;
      gap: 10px;
      text-align: center;
      color: var(--text-muted);
      font-family: var(--font);
      pointer-events: none;
    }
    .empty strong { color: var(--text); font-size: 15px; font-weight: 600; }
    .empty code {
      font-family: var(--mono);
      background: var(--surface-sunken);
      border: 1px solid var(--border);
      border-radius: 4px;
      padding: 1px 5px;
    }
    :host([has-scene]) .empty { display: none; }
  </style>
  <canvas></canvas>
  <div class="empty">
    <strong>No index loaded</strong>
    <div>Open a <code>.cellexport</code> file with File → Open, or drop one anywhere on this window.</div>
    <div>Create one from the runner with <code>cellular --export</code>.</div>
  </div>
`;

export class CellularCanvas extends HTMLElement {
  private canvas!: HTMLCanvasElement;
  private context!: CanvasRenderingContext2D;
  private scene: Scene | null = null;
  private viewport: Viewport = { x: 0, y: 0, scale: 1 };
  private safeArea: SafeArea = { top: 16, right: 16, bottom: 16, left: 16 };
  /** A fit was asked for while the canvas still had no layout. */
  private pendingFit = false;
  private hovered: SceneCell | null = null;
  private pointer = { x: 0, y: 0 };
  private dragging = false;
  private dragMoved = false;
  private dragOrigin = { x: 0, y: 0 };
  private observer?: ResizeObserver;

  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = TEMPLATE;
    this.canvas = root.querySelector('canvas') as HTMLCanvasElement;
    this.context = this.canvas.getContext('2d') as CanvasRenderingContext2D;

    this.observer = new ResizeObserver(() => {
      this.resize();
      this.settlePendingFit();
    });
    this.observer.observe(this);
    this.resize();

    this.canvas.addEventListener('pointerdown', this.onPointerDown);
    this.canvas.addEventListener('pointermove', this.onPointerMove);
    this.canvas.addEventListener('pointerup', this.onPointerUp);
    this.canvas.addEventListener('pointerleave', this.onPointerLeave);
    this.canvas.addEventListener('wheel', this.onWheel, { passive: false });
  }

  disconnectedCallback(): void {
    this.observer?.disconnect();
  }

  setScene(scene: Scene | null, keepViewport = false): void {
    this.scene = scene;
    this.toggleAttribute('has-scene', scene !== null);
    this.hovered = null;
    if (!keepViewport) this.fit();
    else this.draw();
  }

  /** Fit once the element actually has a size, if one was asked for too early. */
  private settlePendingFit(): void {
    if (!this.pendingFit) return;
    const { width, height } = this.canvas.getBoundingClientRect();
    if (width > 0 && height > 0) this.fit();
  }

  /** Reserve room for the controls that sit over the canvas. */
  setSafeArea(area: SafeArea): void {
    this.safeArea = area;
  }

  /** Scale and centre the scene so all of it clears the overlaid controls. */
  fit(): void {
    const scene = this.scene;
    const { width, height } = this.canvas.getBoundingClientRect();
    if (!scene || width === 0 || height === 0) {
      // The element has no layout yet; fit as soon as it gets one.
      this.pendingFit = scene !== null;
      this.draw();
      return;
    }
    this.pendingFit = false;
    const { top, right, bottom, left } = this.safeArea;
    const usableWidth = Math.max(64, width - left - right);
    const usableHeight = Math.max(64, height - top - bottom);
    const scale = Math.min(usableWidth / scene.bounds.width, usableHeight / scene.bounds.height);
    this.viewport.scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));
    this.viewport.x = left + (usableWidth - scene.bounds.width * this.viewport.scale) / 2 - scene.bounds.x * this.viewport.scale;
    this.viewport.y = top + (usableHeight - scene.bounds.height * this.viewport.scale) / 2 - scene.bounds.y * this.viewport.scale;
    this.draw();
  }

  /** Back to 1:1, centred on the scene. */
  resetZoom(): void {
    const scene = this.scene;
    const { width, height } = this.canvas.getBoundingClientRect();
    this.viewport.scale = 1;
    if (scene) {
      const { top, right, bottom, left } = this.safeArea;
      this.viewport.x = left + (width - left - right - scene.bounds.width) / 2 - scene.bounds.x;
      this.viewport.y = top + (height - top - bottom - scene.bounds.height) / 2 - scene.bounds.y;
    } else {
      this.viewport.x = 0;
      this.viewport.y = 0;
    }
    this.draw();
  }

  /** Zoom about a point in client coordinates, defaulting to the pointer. */
  zoomBy(factor: number, clientX?: number, clientY?: number): void {
    const rect = this.canvas.getBoundingClientRect();
    const anchorX = (clientX ?? this.pointer.x + rect.left) - rect.left;
    const anchorY = (clientY ?? this.pointer.y + rect.top) - rect.top;
    const next = Math.min(MAX_SCALE, Math.max(MIN_SCALE, this.viewport.scale * factor));
    const ratio = next / this.viewport.scale;
    this.viewport.x = anchorX - (anchorX - this.viewport.x) * ratio;
    this.viewport.y = anchorY - (anchorY - this.viewport.y) * ratio;
    this.viewport.scale = next;
    this.draw();
    this.emitHover();
  }

  private resize(): void {
    const ratio = window.devicePixelRatio || 1;
    const { width, height } = this.getBoundingClientRect();
    this.canvas.width = Math.max(1, Math.round(width * ratio));
    this.canvas.height = Math.max(1, Math.round(height * ratio));
    this.draw();
  }

  private toWorld(clientX: number, clientY: number): { x: number; y: number } {
    const rect = this.canvas.getBoundingClientRect();
    return {
      x: (clientX - rect.left - this.viewport.x) / this.viewport.scale,
      y: (clientY - rect.top - this.viewport.y) / this.viewport.scale,
    };
  }

  private hitTest(clientX: number, clientY: number): SceneCell | null {
    if (!this.scene) return null;
    const point = this.toWorld(clientX, clientY);
    // Later cells are drawn on top, so search backwards.
    for (let panel = this.scene.panels.length - 1; panel >= 0; panel -= 1) {
      const cells = this.scene.panels[panel].cells;
      for (let index = cells.length - 1; index >= 0; index -= 1) {
        const { rect } = cells[index];
        if (
          point.x >= rect.x &&
          point.x <= rect.x + rect.width &&
          point.y >= rect.y &&
          point.y <= rect.y + rect.height
        ) {
          return cells[index];
        }
      }
    }
    return null;
  }

  private emitHover(): void {
    const cell = this.hitTest(this.pointer.x, this.pointer.y);
    if (cell !== this.hovered) {
      this.hovered = cell;
      this.draw();
    }
    this.dispatchEvent(
      new CustomEvent<HoverDetail>('cell-hover', {
        detail: { cell, clientX: this.pointer.x, clientY: this.pointer.y },
        bubbles: true,
        composed: true,
      }),
    );
  }

  private onPointerDown = (event: PointerEvent): void => {
    if (event.button !== 0) return;
    this.dragging = true;
    this.dragMoved = false;
    this.dragOrigin = { x: event.clientX - this.viewport.x, y: event.clientY - this.viewport.y };
    this.canvas.classList.add('dragging');
    this.canvas.setPointerCapture(event.pointerId);
  };

  private onPointerMove = (event: PointerEvent): void => {
    this.pointer = { x: event.clientX, y: event.clientY };
    if (this.dragging) {
      this.dragMoved = true;
      this.viewport.x = event.clientX - this.dragOrigin.x;
      this.viewport.y = event.clientY - this.dragOrigin.y;
      this.draw();
      return;
    }
    this.emitHover();
  };

  private onPointerUp = (event: PointerEvent): void => {
    if (!this.dragging) return;
    this.dragging = false;
    this.canvas.classList.remove('dragging');
    this.canvas.releasePointerCapture(event.pointerId);
    if (!this.dragMoved) this.emitHover();
  };

  private onPointerLeave = (): void => {
    this.hovered = null;
    this.draw();
    this.dispatchEvent(
      new CustomEvent<HoverDetail>('cell-hover', {
        detail: { cell: null, clientX: 0, clientY: 0 },
        bubbles: true,
        composed: true,
      }),
    );
  };

  private onWheel = (event: WheelEvent): void => {
    event.preventDefault();
    // Trackpad pinch arrives as a wheel event with ctrlKey set.
    const intensity = event.ctrlKey ? 0.012 : 0.0022;
    this.zoomBy(Math.exp(-event.deltaY * intensity), event.clientX, event.clientY);
  };

  // ------------------------------------------------------------ drawing --

  private token(name: string, fallback: string): string {
    const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return value || fallback;
  }

  private draw(): void {
    const context = this.context;
    if (!context) return;
    const ratio = window.devicePixelRatio || 1;
    const width = this.canvas.width / ratio;
    const height = this.canvas.height / ratio;

    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, width, height);
    context.fillStyle = this.token('--bg', '#fbfbfa');
    context.fillRect(0, 0, width, height);
    if (!this.scene) return;

    context.save();
    context.translate(this.viewport.x, this.viewport.y);
    context.scale(this.viewport.scale, this.viewport.scale);

    for (const panel of this.scene.panels) {
      this.drawPanel(context, panel.rect, panel.title, panel.subtitle);
      for (const cell of panel.cells) this.drawCell(context, cell);
    }
    if (this.hovered) this.drawHighlight(context, this.hovered.rect);

    context.restore();
  }

  private drawPanel(
    context: CanvasRenderingContext2D,
    rect: Rect,
    title: string,
    subtitle: string,
  ): void {
    const scale = this.viewport.scale;
    context.save();
    context.fillStyle = this.token('--surface', '#ffffff');
    context.strokeStyle = this.token('--border', '#d9dce1');
    context.lineWidth = 1 / scale;
    roundedRect(context, rect, 10);
    context.fill();
    context.stroke();

    if (rect.height * scale > 60) {
      context.fillStyle = this.token('--text', '#1f2328');
      context.font = `600 ${19}px ${this.token('--mono', 'monospace')}`;
      context.textBaseline = 'alphabetic';
      context.fillText(title, rect.x + 16, rect.y + 27);
      context.fillStyle = this.token('--text-muted', '#656d76');
      context.font = `${14}px ${this.token('--font', 'sans-serif')}`;
      context.fillText(subtitle, rect.x + 16, rect.y + 44);
    }
    context.restore();
  }

  private drawCell(context: CanvasRenderingContext2D, cell: SceneCell): void {
    const scale = this.viewport.scale;
    const { rect } = cell;
    if (rect.width * scale < 0.6 || rect.height * scale < 0.6) return;

    context.save();
    context.globalAlpha = cell.alpha;
    context.fillStyle = cell.fill;
    context.fillRect(rect.x, rect.y, rect.width, rect.height);

    if (cell.slices) {
      // Columns inside the block, one per selected commit.
      const columnWidth = rect.width / cell.slices.length;
      cell.slices.forEach((slice, index) => {
        const columnHeight = rect.height * Math.min(1, Math.max(0, slice.fraction));
        context.fillStyle = slice.colour;
        context.fillRect(
          rect.x + index * columnWidth,
          rect.y + rect.height - columnHeight,
          columnWidth,
          columnHeight,
        );
      });
    }

    context.globalAlpha = 1;
    // Translucent layers are read from their outlines: a white gap between
    // blocks would only wash the stack out.
    const layered = cell.alpha < 1;
    context.strokeStyle = layered ? cell.fill : 'rgba(255, 255, 255, 0.7)';
    context.lineWidth = (layered ? 1.6 : 1) / scale;
    context.strokeRect(rect.x, rect.y, rect.width, rect.height);

    const screenWidth = rect.width * scale;
    const screenHeight = rect.height * scale;
    if (
      cell.showLabel !== false &&
      screenWidth >= LABEL_MIN_WIDTH &&
      screenHeight >= LABEL_MIN_HEIGHT
    ) {
      this.drawLabel(context, cell, screenWidth, screenHeight);
    }
    context.restore();
  }

  private drawLabel(
    context: CanvasRenderingContext2D,
    cell: SceneCell,
    screenWidth: number,
    screenHeight: number,
  ): void {
    const scale = this.viewport.scale;
    const padding = 6 / scale;
    context.save();
    context.beginPath();
    context.rect(cell.rect.x, cell.rect.y, cell.rect.width, cell.rect.height);
    context.clip();
    context.fillStyle = 'rgba(31, 35, 40, 0.86)';
    context.textBaseline = 'top';

    const nameSize = 12 / scale;
    context.font = `600 ${nameSize}px ${this.token('--font', 'sans-serif')}`;
    const label = fitText(context, cell.label, cell.rect.width - padding * 2);
    context.fillText(label, cell.rect.x + padding, cell.rect.y + padding);

    if (screenHeight >= 42 && screenWidth >= 72) {
      context.fillStyle = 'rgba(31, 35, 40, 0.6)';
      context.font = `${11 / scale}px ${this.token('--mono', 'monospace')}`;
      context.fillText(cell.value, cell.rect.x + padding, cell.rect.y + padding + nameSize + 3 / scale);
    }
    context.restore();
  }

  private drawHighlight(context: CanvasRenderingContext2D, rect: Rect): void {
    context.save();
    context.strokeStyle = this.token('--text', '#1f2328');
    context.lineWidth = 2 / this.viewport.scale;
    context.strokeRect(rect.x, rect.y, rect.width, rect.height);
    context.restore();
  }
}

function roundedRect(context: CanvasRenderingContext2D, rect: Rect, radius: number): void {
  const limit = Math.min(radius, rect.width / 2, rect.height / 2);
  context.beginPath();
  context.moveTo(rect.x + limit, rect.y);
  context.arcTo(rect.x + rect.width, rect.y, rect.x + rect.width, rect.y + rect.height, limit);
  context.arcTo(rect.x + rect.width, rect.y + rect.height, rect.x, rect.y + rect.height, limit);
  context.arcTo(rect.x, rect.y + rect.height, rect.x, rect.y, limit);
  context.arcTo(rect.x, rect.y, rect.x + rect.width, rect.y, limit);
  context.closePath();
}

/** Trim text to the available width, ending with an ellipsis when cut. */
function fitText(context: CanvasRenderingContext2D, text: string, maxWidth: number): string {
  if (context.measureText(text).width <= maxWidth) return text;
  let low = 0;
  let high = text.length;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (context.measureText(`${text.slice(0, middle)}…`).width <= maxWidth) low = middle;
    else high = middle - 1;
  }
  return low > 0 ? `${text.slice(0, low)}…` : '';
}

customElements.define('cellular-canvas', CellularCanvas);
