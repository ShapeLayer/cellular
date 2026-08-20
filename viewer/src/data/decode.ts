/**
 * Reading the runner's binary index.
 *
 * The layout is documented in `runner/src/index/format.rs`; this decoder must
 * stay in step with it. All integers are little endian.
 */

import type { Counts, ModuleStats, Snapshot } from '../model';

export const INDEX_MAGIC = 'CELLIDX\0';
export const BLOB_MAGIC = 'CELLBLB\0';
export const FORMAT_VERSION = 2;

export interface BlobMeta {
  size: number;
  md5: Uint8Array;
}

class Reader {
  private view: DataView;
  private at = 0;

  constructor(private bytes: Uint8Array) {
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  get done(): boolean {
    return this.at >= this.bytes.length;
  }

  take(count: number): Uint8Array {
    const end = this.at + count;
    if (end > this.bytes.length) throw new Error('the index record is truncated');
    const slice = this.bytes.subarray(this.at, end);
    this.at = end;
    return slice;
  }

  u8(): number {
    return this.view.getUint8(this.bump(1));
  }
  u16(): number {
    return this.view.getUint16(this.bump(2), true);
  }
  u32(): number {
    return this.view.getUint32(this.bump(4), true);
  }
  i32(): number {
    return this.view.getInt32(this.bump(4), true);
  }
  u64(): number {
    return Number(this.view.getBigUint64(this.bump(8), true));
  }
  i64(): number {
    return Number(this.view.getBigInt64(this.bump(8), true));
  }

  ascii(count: number): string {
    return String.fromCharCode(...this.take(count));
  }

  string(): string {
    return new TextDecoder().decode(this.take(this.u16()));
  }

  hex(count: number): string {
    let out = '';
    for (const byte of this.take(count)) out += byte.toString(16).padStart(2, '0');
    return out;
  }

  private bump(size: number): number {
    const at = this.at;
    if (at + size > this.bytes.length) throw new Error('the index record is truncated');
    this.at += size;
    return at;
  }
}

export function decodeIndex(bytes: Uint8Array): BlobMeta[] {
  const reader = new Reader(bytes);
  if (reader.ascii(8) !== INDEX_MAGIC) throw new Error('not a Cellular INDEX file');
  const version = reader.u16();
  if (version !== FORMAT_VERSION) {
    throw new Error(
      `this index was written in format version ${version}, but the viewer reads version ${FORMAT_VERSION}`,
    );
  }
  reader.u16();
  const count = reader.u32();
  const blobs: BlobMeta[] = [];
  for (let index = 0; index < count; index += 1) {
    const size = reader.u64();
    blobs.push({ size, md5: reader.take(16).slice() });
  }
  if (!reader.done) throw new Error('the INDEX file has trailing bytes');
  return blobs;
}

/** Split a blob container into raw record payloads. */
export function decodeBlob(bytes: Uint8Array): Uint8Array[] {
  const reader = new Reader(bytes);
  if (reader.ascii(8) !== BLOB_MAGIC) throw new Error('not a Cellular blob file');
  const version = reader.u16();
  if (version !== FORMAT_VERSION) {
    throw new Error(`unsupported blob format version ${version}`);
  }
  reader.u16();
  const count = reader.u32();
  const records: Uint8Array[] = [];
  for (let index = 0; index < count; index += 1) {
    records.push(reader.take(reader.u32()));
  }
  return records;
}

export function decodeSnapshot(payload: Uint8Array): Snapshot {
  const reader = new Reader(payload);
  const oid = reader.hex(reader.u8());
  const commitTime = reader.i64();
  const tzOffset = reader.i32();
  const spec = reader.string();
  const summary = reader.string();
  const author = reader.string();

  const parents: string[] = [];
  const parentCount = reader.u8();
  for (let index = 0; index < parentCount; index += 1) {
    parents.push(reader.hex(reader.u8()));
  }

  const refs: string[] = [];
  const refCount = reader.u16();
  for (let index = 0; index < refCount; index += 1) refs.push(reader.string());

  const indexDepth = reader.u32();
  const metrics = reader.u8();
  // The scan fingerprint decides whether the runner may reuse a record; the
  // viewer only has to step over it.
  reader.take(16);

  const languageTable: string[] = [];
  const languageCount = reader.u32();
  for (let index = 0; index < languageCount; index += 1) languageTable.push(reader.string());

  const modules: ModuleStats[] = [];
  const moduleCount = reader.u32();
  for (let index = 0; index < moduleCount; index += 1) {
    const path = reader.string();
    const totals: Counts = { files: reader.u64(), lines: reader.u64(), chars: reader.u64() };
    const languages = new Map<string, Counts>();
    const entryCount = reader.u32();
    for (let entry = 0; entry < entryCount; entry += 1) {
      const name = languageTable[reader.u32()];
      if (name === undefined) throw new Error('the record references an unknown language id');
      languages.set(name, { files: reader.u64(), lines: reader.u64(), chars: reader.u64() });
    }
    modules.push({ path, totals, languages });
  }

  return {
    oid,
    parents,
    refs,
    commitTime,
    tzOffset,
    spec,
    summary,
    author,
    indexDepth,
    metrics,
    modules,
  };
}
