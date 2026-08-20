/**
 * MD5, so the viewer can check a blob against the digest the runner recorded
 * in `INDEX`. `crypto.subtle` does not offer MD5, and the index format uses it,
 * so it lives here.
 *
 * Based on RFC 1321. Operates on bytes and returns 16 bytes.
 */

const SHIFTS = [
  7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14,
  20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6,
  10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const SINE = new Uint32Array(64);
for (let index = 0; index < 64; index += 1) {
  SINE[index] = Math.floor(Math.abs(Math.sin(index + 1)) * 0x1_0000_0000);
}

function rotateLeft(value: number, by: number): number {
  return (value << by) | (value >>> (32 - by));
}

export function md5(bytes: Uint8Array): Uint8Array {
  // Message, a 0x80 byte, zero padding to 56 mod 64, then the bit length.
  const padded = new Uint8Array((((bytes.length + 8) >> 6) + 1) << 6);
  padded.set(bytes);
  padded[bytes.length] = 0x80;
  const view = new DataView(padded.buffer);
  const bitLength = bytes.length * 8;
  view.setUint32(padded.length - 8, bitLength >>> 0, true);
  view.setUint32(padded.length - 4, Math.floor(bitLength / 0x1_0000_0000), true);

  let a0 = 0x67452301;
  let b0 = 0xefcdab89;
  let c0 = 0x98badcfe;
  let d0 = 0x10325476;

  const block = new Uint32Array(16);
  for (let offset = 0; offset < padded.length; offset += 64) {
    for (let word = 0; word < 16; word += 1) {
      block[word] = view.getUint32(offset + word * 4, true);
    }

    let a = a0;
    let b = b0;
    let c = c0;
    let d = d0;

    for (let step = 0; step < 64; step += 1) {
      let mixed: number;
      let index: number;
      if (step < 16) {
        mixed = (b & c) | (~b & d);
        index = step;
      } else if (step < 32) {
        mixed = (d & b) | (~d & c);
        index = (5 * step + 1) % 16;
      } else if (step < 48) {
        mixed = b ^ c ^ d;
        index = (3 * step + 5) % 16;
      } else {
        mixed = c ^ (b | ~d);
        index = (7 * step) % 16;
      }
      const sum = (a + mixed + SINE[step] + block[index]) | 0;
      a = d;
      d = c;
      c = b;
      b = (b + rotateLeft(sum, SHIFTS[step])) | 0;
    }

    a0 = (a0 + a) | 0;
    b0 = (b0 + b) | 0;
    c0 = (c0 + c) | 0;
    d0 = (d0 + d) | 0;
  }

  const digest = new Uint8Array(16);
  const out = new DataView(digest.buffer);
  out.setUint32(0, a0 >>> 0, true);
  out.setUint32(4, b0 >>> 0, true);
  out.setUint32(8, c0 >>> 0, true);
  out.setUint32(12, d0 >>> 0, true);
  return digest;
}

export function toHex(bytes: Uint8Array): string {
  let out = '';
  for (const byte of bytes) out += byte.toString(16).padStart(2, '0');
  return out;
}

export function sameDigest(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}
