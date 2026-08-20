/**
 * Turning selected snapshots into a laid-out, world-space scene.
 *
 * Everything here works in world coordinates; the canvas applies the pan and
 * zoom transform on top, so the layout does not change while navigating.
 */

import { squarify, type Rect } from '../layout/squarify';
import {
  dominantLanguage,
  measure,
  snapshotTotal,
  type Metric,
  type ModuleStats,
  type Snapshot,
  METRIC_LABELS,
  formatCommitTime,
  shortOid,
} from '../model';
import { DELTA, DELTA_LABELS, languageColour, layerColour } from './palette';
import type { Settings } from '../state/store';

export const PANEL_WIDTH = 920;
export const PANEL_HEIGHT = 640;
export const PANEL_GAP = 56;
export const PANEL_HEADER = 52;
export const PANEL_PADDING = 14;

export interface TooltipContent {
  title: string;
  subtitle?: string;
  rows: Array<[string, string]>;
}

export interface CellSlice {
  /** Share of the block height this slice fills, 0 to 1. */
  fraction: number;
  colour: string;
}

export interface SceneCell {
  rect: Rect;
  fill: string;
  alpha: number;
  label: string;
  value: string;
  slices?: CellSlice[];
  /** Layers under the top one stay unlabelled, or the text piles up. */
  showLabel?: boolean;
  tooltip: TooltipContent;
}

export interface ScenePanel {
  rect: Rect;
  title: string;
  subtitle: string;
  cells: SceneCell[];
}

export interface LegendEntry {
  colour: string;
  label: string;
}

export interface Scene {
  panels: ScenePanel[];
  bounds: Rect;
  legend: LegendEntry[];
  /** Shown when a mode needs a word of explanation. */
  note: string;
}

export function formatValue(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}k`;
  return String(Math.round(value));
}

function signed(value: number): string {
  return value > 0 ? `+${formatValue(value)}` : value < 0 ? `-${formatValue(-value)}` : '0';
}

function moduleTooltip(
  snapshot: Snapshot,
  module: ModuleStats,
  metric: Metric,
): TooltipContent {
  const languages = [...module.languages.entries()]
    .sort((left, right) => measure(right[1], metric) - measure(left[1], metric))
    .slice(0, 6)
    .map(([name, counts]) => [name, formatValue(measure(counts, metric))] as [string, string]);
  return {
    title: module.path,
    subtitle: `${shortOid(snapshot.oid)} · ${snapshot.summary}`,
    rows: [
      ['Files', formatValue(module.totals.files)],
      ['Lines', formatValue(module.totals.lines)],
      ['Characters', formatValue(module.totals.chars)],
      ...(languages.length > 0 ? ([['—', '']] as Array<[string, string]>) : []),
      ...languages,
    ],
  };
}

function panelFrames(count: number): Rect[] {
  const columns = Math.max(1, Math.ceil(Math.sqrt(count)));
  const frames: Rect[] = [];
  for (let index = 0; index < count; index += 1) {
    const column = index % columns;
    const row = Math.floor(index / columns);
    frames.push({
      x: column * (PANEL_WIDTH + PANEL_GAP),
      y: row * (PANEL_HEIGHT + PANEL_GAP),
      width: PANEL_WIDTH,
      height: PANEL_HEIGHT,
    });
  }
  return frames;
}

function plotArea(frame: Rect): Rect {
  return {
    x: frame.x + PANEL_PADDING,
    y: frame.y + PANEL_HEADER,
    width: frame.width - PANEL_PADDING * 2,
    height: frame.height - PANEL_HEADER - PANEL_PADDING,
  };
}

/** Scale a plot area so its area is proportional to `share`, keeping it centred. */
function scaled(area: Rect, share: number): Rect {
  const factor = Math.sqrt(Math.max(share, 0));
  if (!Number.isFinite(factor)) return area;
  const width = area.width * factor;
  const height = area.height * factor;
  return {
    x: area.x + (area.width - width) / 2,
    y: area.y + (area.height - height) / 2,
    width,
    height,
  };
}

function boundsOf(panels: ScenePanel[]): Rect {
  if (panels.length === 0) return { x: 0, y: 0, width: PANEL_WIDTH, height: PANEL_HEIGHT };
  const rects = panels.flatMap((panel) => [panel.rect, ...panel.cells.map((cell) => cell.rect)]);
  const left = Math.min(...rects.map((rect) => rect.x));
  const top = Math.min(...rects.map((rect) => rect.y));
  const right = Math.max(...rects.map((rect) => rect.x + rect.width));
  const bottom = Math.max(...rects.map((rect) => rect.y + rect.height));
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** The selected endpoint which occupies the normal, full plot area. */
function referenceTotal(snapshots: Snapshot[], metric: Metric, settings: Settings): number {
  const reference =
    settings.multiCommitSizeReference === 'oldest' ? snapshots[0] : snapshots[snapshots.length - 1];
  return Math.max(1, snapshotTotal(reference, metric));
}

function panelTitle(snapshot: Snapshot): string {
  const refs = snapshot.refs.length > 0 ? ` (${snapshot.refs.join(', ')})` : '';
  return `${shortOid(snapshot.oid)}${refs}`;
}

function panelSubtitle(snapshot: Snapshot, metric: Metric): string {
  return `${formatCommitTime(snapshot)} · ${formatValue(snapshotTotal(snapshot, metric))} ${METRIC_LABELS[
    metric
  ].toLowerCase()}`;
}

function languageLegend(snapshots: Snapshot[], metric: Metric): LegendEntry[] {
  const totals = new Map<string, number>();
  for (const snapshot of snapshots) {
    for (const module of snapshot.modules) {
      for (const [name, counts] of module.languages) {
        totals.set(name, (totals.get(name) ?? 0) + measure(counts, metric));
      }
    }
  }
  return [...totals.entries()]
    .sort((left, right) => right[1] - left[1])
    .slice(0, 10)
    .map(([name]) => ({ colour: languageColour(name), label: name }));
}

/** One treemap per commit, laid out in a grid. */
function buildSideBySide(snapshots: Snapshot[], metric: Metric, settings: Settings): Scene {
  const frames = panelFrames(snapshots.length);
  const totals = snapshots.map((snapshot) => snapshotTotal(snapshot, metric));
  const reference = referenceTotal(snapshots, metric, settings);

  const panels = snapshots.map((snapshot, index) => {
    const area = scaled(plotArea(frames[index]), totals[index] / reference);
    const cells = squarify(
      snapshot.modules.map((module) => ({ value: measure(module.totals, metric), data: module })),
      area,
    ).map<SceneCell>((cell) => ({
      rect: cell,
      fill: languageColour(dominantLanguage(cell.data, metric)),
      alpha: 1,
      label: cell.data.path,
      value: formatValue(measure(cell.data.totals, metric)),
      tooltip: moduleTooltip(snapshot, cell.data, metric),
    }));
    return {
      rect: frames[index],
      title: panelTitle(snapshot),
      subtitle: panelSubtitle(snapshot, metric),
      cells,
    };
  });

  const scene = {
    panels,
    bounds: boundsOf(panels),
    legend: languageLegend(snapshots, metric),
    note:
      snapshots.length > 1
        ? 'Panel size follows each commit’s total, so growth is visible between panels.'
        : '',
  };
  return scene;
}

interface UnionEntry {
  path: string;
  values: number[];
  modules: Array<ModuleStats | undefined>;
}

/** Every module seen in any selected commit, with its value in each. */
function unionModules(snapshots: Snapshot[], metric: Metric): UnionEntry[] {
  const entries = new Map<string, UnionEntry>();
  snapshots.forEach((snapshot, index) => {
    for (const module of snapshot.modules) {
      let entry = entries.get(module.path);
      if (!entry) {
        entry = {
          path: module.path,
          values: new Array(snapshots.length).fill(0),
          modules: new Array(snapshots.length).fill(undefined),
        };
        entries.set(module.path, entry);
      }
      entry.values[index] = measure(module.totals, metric);
      entry.modules[index] = module;
    }
  });
  return [...entries.values()];
}

/** One layout, coloured by what changed between the first and last commit. */
function buildChangeHighlight(snapshots: Snapshot[], metric: Metric, settings: Settings): Scene {
  const oldest = snapshots[0];
  const newest = snapshots[snapshots.length - 1];
  const entries = unionModules([oldest, newest], metric);
  const frame = panelFrames(1)[0];

  const area = scaled(
    plotArea(frame),
    Math.max(snapshotTotal(oldest, metric), snapshotTotal(newest, metric)) /
      referenceTotal(snapshots, metric, settings),
  );
  const cells = squarify(
    entries.map((entry) => ({ value: Math.max(entry.values[0], entry.values[1]), data: entry })),
    area,
  ).map<SceneCell>((cell) => {
    const [before, after] = cell.data.values;
    const delta = after - before;
    let kind: keyof typeof DELTA;
    if (before === 0 && after > 0) kind = 'added';
    else if (after === 0 && before > 0) kind = 'removed';
    else if (delta > before * 0.02) kind = 'grown';
    else if (delta < -before * 0.02) kind = 'shrunk';
    else kind = 'unchanged';

    return {
      rect: cell,
      fill: DELTA[kind],
      alpha: 1,
      label: cell.data.path,
      value: signed(delta),
      tooltip: {
        title: cell.data.path,
        subtitle: `${shortOid(oldest.oid)} → ${shortOid(newest.oid)} · ${DELTA_LABELS[kind]}`,
        rows: [
          [shortOid(oldest.oid), formatValue(before)],
          [shortOid(newest.oid), formatValue(after)],
          ['Change', signed(delta)],
        ],
      },
    };
  });

  const panels = [
      {
        rect: frame,
        title: `${shortOid(oldest.oid)} → ${shortOid(newest.oid)}`,
        subtitle:
          snapshots.length > 2
            ? `Change in ${METRIC_LABELS[metric].toLowerCase()} from the oldest to the newest of ${snapshots.length} selected commits`
            : `Change in ${METRIC_LABELS[metric].toLowerCase()} between the two selected commits`,
        cells,
      },
    ];
  return {
    panels,
    bounds: boundsOf(panels),
    legend: (Object.keys(DELTA) as Array<keyof typeof DELTA>).map((kind) => ({
      colour: DELTA[kind],
      label: DELTA_LABELS[kind],
    })),
    note: 'Blocks are sized by the larger of the two commits, so removed modules stay visible.',
  };
}

/** Each commit's own treemap, stacked with transparency. */
function buildTranslucentLayers(snapshots: Snapshot[], metric: Metric, settings: Settings): Scene {
  const frame = panelFrames(1)[0];
  const area = plotArea(frame);
  const totals = snapshots.map((snapshot) => snapshotTotal(snapshot, metric));
  const reference = referenceTotal(snapshots, metric, settings);
  // Layers of nearly identical treemaps would blend into one flat colour, so
  // the fill stays faint and each layer is read from its outline instead.
  const alpha = Math.min(0.3, Math.max(0.14, 0.6 / snapshots.length));

  const cells: SceneCell[] = [];
  snapshots.forEach((snapshot, index) => {
    const layer = scaled(area, totals[index] / reference);
    const top = index === snapshots.length - 1;
    for (const cell of squarify(
      snapshot.modules.map((module) => ({ value: measure(module.totals, metric), data: module })),
      layer,
    )) {
      cells.push({
        rect: cell,
        fill: layerColour(index),
        alpha,
        showLabel: top,
        label: cell.data.path,
        value: formatValue(measure(cell.data.totals, metric)),
        tooltip: moduleTooltip(snapshot, cell.data, metric),
      });
    }
  });

  const panels = [
      {
        rect: frame,
        title: `${snapshots.length} commits layered`,
        subtitle: `${shortOid(snapshots[0].oid)} → ${shortOid(snapshots[snapshots.length - 1].oid)}`,
        cells,
      },
    ];
  return {
    panels,
    bounds: boundsOf(panels),
    legend: snapshots.map((snapshot, index) => ({
      colour: layerColour(index),
      label: `${shortOid(snapshot.oid)} · ${snapshot.summary.slice(0, 28)}`,
    })),
    note: 'Each commit keeps its own layout; the outlines show where the blocks moved.',
  };
}

/** One layout, with every block divided into a column per commit. */
function buildSplitCells(snapshots: Snapshot[], metric: Metric, settings: Settings): Scene {
  const entries = unionModules(snapshots, metric);
  const frame = panelFrames(1)[0];

  const area = scaled(plotArea(frame), Math.max(...snapshots.map((snapshot) => snapshotTotal(snapshot, metric))) /
    referenceTotal(snapshots, metric, settings));
  const cells = squarify(
    entries.map((entry) => ({ value: Math.max(...entry.values), data: entry })),
    area,
  ).map<SceneCell>((cell) => {
    const peak = Math.max(...cell.data.values, 1);
    return {
      rect: cell,
      fill: '#eef0f2',
      alpha: 1,
      label: cell.data.path,
      value: formatValue(peak),
      slices: cell.data.values.map((value, index) => ({
        fraction: value / peak,
        colour: layerColour(index),
      })),
      tooltip: {
        title: cell.data.path,
        subtitle: `${METRIC_LABELS[metric]} per selected commit`,
        rows: snapshots.map(
          (snapshot, index) =>
            [shortOid(snapshot.oid), formatValue(cell.data.values[index])] as [string, string],
        ),
      },
    };
  });

  const panels = [
      {
        rect: frame,
        title: `${snapshots.length} commits compared`,
        subtitle: `Each block is split into one column per commit, by ${METRIC_LABELS[
          metric
        ].toLowerCase()}`,
        cells,
      },
    ];
  return {
    panels,
    bounds: boundsOf(panels),
    legend: snapshots.map((snapshot, index) => ({
      colour: layerColour(index),
      label: `${shortOid(snapshot.oid)} · ${snapshot.summary.slice(0, 28)}`,
    })),
    note: 'Block size follows the largest of the selected commits.',
  };
}

export function buildScene(snapshots: Snapshot[], settings: Settings): Scene | null {
  if (snapshots.length === 0) return null;
  if (snapshots.length === 1 || settings.multiCommitMode === 'side-by-side') {
    return buildSideBySide(snapshots, settings.metric, settings);
  }
  switch (settings.overlayStyle) {
    case 'translucent-layers':
      return buildTranslucentLayers(snapshots, settings.metric, settings);
    case 'split-cells':
      return buildSplitCells(snapshots, settings.metric, settings);
    default:
      return buildChangeHighlight(snapshots, settings.metric, settings);
  }
}
