/**
 * Opening a `.cellexport` file.
 *
 * The runner writes an 8-byte magic and a version in front of a ZIP archive
 * holding `INDEX` and `BLOBS/BLOB_#`. The magic is checked first so a file
 * that is not an index is refused before anything tries to unpack it.
 */

import { unzipSync } from 'fflate';

import { decodeBlob, decodeIndex, decodeSnapshot } from './decode';
import { md5, sameDigest, toHex } from './md5';
import { orderOldestFirst } from './order';
import type { Snapshot } from '../model';

export const EXPORT_MAGIC = 'CELLEXP\0';
export const EXPORT_VERSION = 1;
export const EXPORT_EXTENSION = '.cellexport';

export interface LoadedIndex {
  snapshots: Snapshot[];
  blobCount: number;
  /** Anything that was wrong but did not stop the file from loading. */
  warnings: string[];
}

function ascii(bytes: Uint8Array, from: number, length: number): string {
  return String.fromCharCode(...bytes.subarray(from, from + length));
}

export function readCellExport(bytes: Uint8Array): LoadedIndex {
  if (bytes.length < 10 || ascii(bytes, 0, 8) !== EXPORT_MAGIC) {
    throw new Error(
      'this file is not a Cellular export; build one with `cellular --export` or `index export` in the runner',
    );
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const version = view.getUint16(8, true);
  if (version !== EXPORT_VERSION) {
    throw new Error(`this export was written in version ${version}, which the viewer cannot read`);
  }

  const files = unzipSync(bytes.subarray(10));
  const indexFile = files['INDEX'];
  if (!indexFile) throw new Error('the export has no INDEX entry');

  const warnings: string[] = [];
  const metas = decodeIndex(indexFile);
  const snapshots: Snapshot[] = [];

  for (let number = 0; number < metas.length; number += 1) {
    const name = `BLOBS/BLOB_${number}`;
    const blob = files[name];
    if (!blob) {
      warnings.push(`${name} is missing from the export`);
      continue;
    }
    if (blob.length !== metas[number].size) {
      warnings.push(
        `${name} is ${blob.length} bytes, but INDEX expects ${metas[number].size}; it may be damaged`,
      );
      continue;
    }
    // The same check the runner's --verify makes. A blob that fails it is not
    // partially read: whatever parses out of damaged bytes would be fiction.
    const digest = md5(blob);
    if (!sameDigest(digest, metas[number].md5)) {
      warnings.push(
        `${name} has md5 ${toHex(digest)}, but INDEX expects ${toHex(
          metas[number].md5,
        )}; it is damaged and was skipped`,
      );
      continue;
    }
    let records: Uint8Array[];
    try {
      records = decodeBlob(blob);
    } catch (error) {
      warnings.push(`${name} could not be read: ${(error as Error).message}`);
      continue;
    }
    let unreadable = 0;
    for (const record of records) {
      try {
        snapshots.push(decodeSnapshot(record));
      } catch {
        unreadable += 1;
      }
    }
    if (unreadable > 0) {
      warnings.push(`${unreadable} record(s) in ${name} could not be read`);
    }
  }

  if (snapshots.length === 0) {
    // Say why, or the reader is left with a bare refusal.
    throw new Error(
      warnings.length > 0
        ? `the export holds no readable snapshots — ${warnings.join('; ')}`
        : 'the export holds no readable snapshots',
    );
  }

  return { snapshots: orderOldestFirst(snapshots), blobCount: metas.length, warnings };
}

export async function readCellExportFile(file: File): Promise<LoadedIndex> {
  const buffer = await file.arrayBuffer();
  return readCellExport(new Uint8Array(buffer));
}
