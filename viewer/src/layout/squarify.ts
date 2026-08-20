/**
 * Squarified treemap layout.
 *
 * Blocks take an area proportional to their value and are packed into rows so
 * they stay close to square rather than stretching into long strips.
 */

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface TreemapItem<T> {
  value: number;
  data: T;
}

export interface TreemapCell<T> extends Rect {
  data: T;
}

function worst(row: number[], side: number, scale: number): number {
  let sum = 0;
  let min = Infinity;
  let max = 0;
  for (const value of row) {
    sum += value;
    if (value < min) min = value;
    if (value > max) max = value;
  }
  const area = sum * scale;
  if (area === 0 || side === 0) return Infinity;
  return Math.max((side * side * max * scale) / (area * area), (area * area) / (side * side * min * scale));
}

/**
 * Lay `items` out inside `bounds`. Items with a value of zero or less are
 * dropped, since a block with no area cannot be seen or hovered.
 */
export function squarify<T>(items: TreemapItem<T>[], bounds: Rect): TreemapCell<T>[] {
  const positive = items.filter((item) => item.value > 0);
  if (positive.length === 0 || bounds.width <= 0 || bounds.height <= 0) return [];

  const sorted = [...positive].sort((left, right) => right.value - left.value);
  const total = sorted.reduce((sum, item) => sum + item.value, 0);
  const scale = (bounds.width * bounds.height) / total;

  const cells: TreemapCell<T>[] = [];
  let free: Rect = { ...bounds };
  let cursor = 0;

  while (cursor < sorted.length) {
    const side = Math.min(free.width, free.height);
    const row: number[] = [];
    const rowItems: TreemapItem<T>[] = [];

    // Grow the row while it makes the blocks squarer.
    while (cursor < sorted.length) {
      const candidate = sorted[cursor];
      const next = [...row, candidate.value];
      if (row.length > 0 && worst(next, side, scale) > worst(row, side, scale)) break;
      row.push(candidate.value);
      rowItems.push(candidate);
      cursor += 1;
    }

    const rowArea = row.reduce((sum, value) => sum + value, 0) * scale;
    const thickness = side === 0 ? 0 : rowArea / side;
    let offset = 0;

    if (free.width >= free.height) {
      for (let index = 0; index < rowItems.length; index += 1) {
        const height = (row[index] * scale) / thickness;
        cells.push({
          x: free.x,
          y: free.y + offset,
          width: thickness,
          height,
          data: rowItems[index].data,
        });
        offset += height;
      }
      free = { x: free.x + thickness, y: free.y, width: free.width - thickness, height: free.height };
    } else {
      for (let index = 0; index < rowItems.length; index += 1) {
        const width = (row[index] * scale) / thickness;
        cells.push({
          x: free.x + offset,
          y: free.y,
          width,
          height: thickness,
          data: rowItems[index].data,
        });
        offset += width;
      }
      free = { x: free.x, y: free.y + thickness, width: free.width, height: free.height - thickness };
    }
  }

  return cells;
}
