/** Viewer settings, kept across sessions in local storage. */

import type { Metric } from '../model';

export type MultiCommitMode = 'side-by-side' | 'overlay';
export type OverlayStyle = 'change-highlight' | 'translucent-layers' | 'split-cells';
/** Which end of a multi-commit selection defines a full-size heatmap. */
export type MultiCommitSizeReference = 'newest' | 'oldest';

export interface Settings {
  multiCommitMode: MultiCommitMode;
  multiCommitSizeReference: MultiCommitSizeReference;
  overlayStyle: OverlayStyle;
  metric: Metric;
  timelineVisible: boolean;
}

const STORAGE_KEY = 'cellular.viewer.settings';

export const DEFAULT_SETTINGS: Settings = {
  multiCommitMode: 'side-by-side',
  multiCommitSizeReference: 'newest',
  overlayStyle: 'change-highlight',
  metric: 'lines',
  timelineVisible: true,
};

const MULTI_MODES: MultiCommitMode[] = ['side-by-side', 'overlay'];
const MULTI_COMMIT_SIZE_REFERENCES: MultiCommitSizeReference[] = ['newest', 'oldest'];
const OVERLAY_STYLES: OverlayStyle[] = ['change-highlight', 'translucent-layers', 'split-cells'];
const METRICS: Metric[] = ['lines', 'chars', 'files'];

export function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULT_SETTINGS };
    const stored = JSON.parse(raw) as Partial<Settings>;
    return {
      multiCommitMode: MULTI_MODES.includes(stored.multiCommitMode as MultiCommitMode)
        ? (stored.multiCommitMode as MultiCommitMode)
        : DEFAULT_SETTINGS.multiCommitMode,
      multiCommitSizeReference: MULTI_COMMIT_SIZE_REFERENCES.includes(
        stored.multiCommitSizeReference as MultiCommitSizeReference,
      )
        ? (stored.multiCommitSizeReference as MultiCommitSizeReference)
        : DEFAULT_SETTINGS.multiCommitSizeReference,
      overlayStyle: OVERLAY_STYLES.includes(stored.overlayStyle as OverlayStyle)
        ? (stored.overlayStyle as OverlayStyle)
        : DEFAULT_SETTINGS.overlayStyle,
      metric: METRICS.includes(stored.metric as Metric)
        ? (stored.metric as Metric)
        : DEFAULT_SETTINGS.metric,
      timelineVisible:
        typeof stored.timelineVisible === 'boolean'
          ? stored.timelineVisible
          : DEFAULT_SETTINGS.timelineVisible,
    };
  } catch {
    // A browser with storage disabled still gets a working viewer.
    return { ...DEFAULT_SETTINGS };
  }
}

export function saveSettings(settings: Settings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    /* storage is unavailable; the session still works, it just will not persist */
  }
}
