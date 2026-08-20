/**
 * Block colours.
 *
 * A block is tinted by the language that dominates its module. The tones stay
 * muted so a screen full of them still reads as a document rather than a
 * poster, and so black text keeps its contrast on every one of them.
 */

const CURATED: Record<string, string> = {
  Rust: '#c9906d',
  C: '#93a6c4',
  'C++': '#8496c2',
  'C#': '#909cd1',
  Java: '#c59274',
  Kotlin: '#a68dc5',
  Swift: '#d89478',
  'Objective-C': '#90a9cd',
  Go: '#7fb1c5',
  Python: '#7fa9cd',
  Ruby: '#ca8b93',
  PHP: '#9b94c5',
  Perl: '#aaa2c1',
  Lua: '#90a1cd',
  Dart: '#79b3b7',
  Scala: '#cd908b',
  Haskell: '#a190c1',
  Elixir: '#a393c3',
  Erlang: '#c190a9',
  Clojure: '#90b69c',
  Zig: '#d1a375',
  Nim: '#d4c180',
  Julia: '#aa93c7',
  R: '#90a7cd',
  SQL: '#9aadb9',
  Shell: '#94b699',
  PowerShell: '#90a5cd',
  Batch: '#b1b1a9',
  JavaScript: '#d1bc75',
  'JavaScript (JSX)': '#d9c790',
  TypeScript: '#809bcc',
  'TypeScript (TSX)': '#94abd7',
  Vue: '#87bea1',
  Svelte: '#d69381',
  HTML: '#d89b81',
  CSS: '#90aad3',
  Sass: '#ca94b7',
  Less: '#909fc1',
  USS: '#a0aed6',
  UXML: '#b6a9d0',
  XML: '#a4b1be',
  TOML: '#b4a995',
  INI: '#b4ad9d',
  Gradle: '#aab690',
  HLSL: '#a3b1d1',
  GLSL: '#a1bdc1',
  WGSL: '#99b9c9',
  Metal: '#b19fcc',
  'Unity Assembly Definition': '#b9b191',
  'Protocol Buffers': '#a0b4a9',
  GraphQL: '#ca90b5',
  Terraform: '#a999cd',
  'Vim Script': '#91b69c',
  Assembly: '#b2a99f',
  Fortran: '#b594b1',
  'Visual Basic': '#90a3c9',
  Pascal: '#c2a285',
  D: '#c18c8c',
  Crystal: '#a9a3af',
  Solidity: '#a6a6ae',
  Makefile: '#b1a790',
  Dockerfile: '#90aac9',
  CMake: '#a6b193',
};

/** Blocks whose module has no language information at all. */
export const UNKNOWN_FILL = '#c3c7cc';

function hash(text: string): number {
  let value = 0;
  for (let index = 0; index < text.length; index += 1) {
    value = (value * 31 + text.charCodeAt(index)) >>> 0;
  }
  return value;
}

/**
 * A stable, muted colour for any language name. Unknown extensions such as
 * `.bin` land on the generated wheel rather than all sharing one grey.
 */
export function languageColour(language: string | null): string {
  if (!language) return UNKNOWN_FILL;
  const curated = CURATED[language];
  if (curated) return curated;
  const hue = hash(language) % 360;
  return `hsl(${hue}, 30%, 71%)`;
}

/** Colours for the change highlight overlay, in the same muted register. */
export const DELTA = {
  added: '#8fbb9b',
  grown: '#b6cfa4',
  unchanged: '#d3d7dc',
  shrunk: '#e3bfa0',
  removed: '#d8a3a3',
};

export const DELTA_LABELS: Record<keyof typeof DELTA, string> = {
  added: 'added',
  grown: 'grown',
  unchanged: 'unchanged',
  shrunk: 'shrunk',
  removed: 'removed',
};

/** Distinct, muted tints for the layers of a translucent overlay. */
const LAYERS = ['#7f9acb', '#c9906d', '#90b69c', '#aa93c7', '#d1bc75', '#ca8b93'];

export function layerColour(index: number): string {
  return LAYERS[index % LAYERS.length];
}
