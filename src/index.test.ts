import { describe, expect, it } from 'vitest';

import { NAME, VERSION, slugify } from './index.js';

describe('package metadata', () => {
  it('exposes a name and semver version', () => {
    expect(NAME).toBe('inkwell');
    expect(VERSION).toMatch(/^\d+\.\d+\.\d+$/);
  });
});

describe('slugify', () => {
  it('lowercases and hyphenates words', () => {
    expect(slugify('Hello World')).toBe('hello-world');
  });

  it('strips punctuation and collapses separators', () => {
    expect(slugify('Hello, World!  Again??')).toBe('hello-world-again');
  });

  it('trims leading and trailing separators', () => {
    expect(slugify('  --Draft--  ')).toBe('draft');
  });

  it('removes diacritics', () => {
    expect(slugify('Crème Brûlée')).toBe('creme-brulee');
  });

  it('returns an empty string for input with no alphanumerics', () => {
    expect(slugify('!!')).toBe('');
  });
});
