import { readFile } from 'node:fs/promises';

const files = {
  'en-US': new URL('../src/core/i18n/locales/en-US.json', import.meta.url),
  'zh-CN': new URL('../src/core/i18n/locales/zh-CN.json', import.meta.url),
};

const dictionaries = Object.fromEntries(
  await Promise.all(
    Object.entries(files).map(async ([locale, url]) => [
      locale,
      JSON.parse(await readFile(url, 'utf8')),
    ]),
  ),
);
const reference = dictionaries['en-US'];
const referenceKeys = Object.keys(reference).sort();
const placeholders = (value) =>
  [...value.matchAll(/\{\{\s*([\w.-]+)\s*\}\}/g)].map((match) => match[1]).sort();
const errors = [];

for (const [locale, dictionary] of Object.entries(dictionaries)) {
  const keys = Object.keys(dictionary).sort();
  for (const key of referenceKeys.filter((key) => !keys.includes(key)))
    errors.push(`${locale}: missing ${key}`);
  for (const key of keys.filter((key) => !referenceKeys.includes(key)))
    errors.push(`${locale}: extra ${key}`);
  for (const key of keys) {
    if (typeof dictionary[key] !== 'string' || dictionary[key].trim() === '') {
      errors.push(`${locale}: empty ${key}`);
      continue;
    }
    if (
      reference[key] &&
      placeholders(dictionary[key]).join(',') !== placeholders(reference[key]).join(',')
    ) {
      errors.push(`${locale}: placeholder mismatch ${key}`);
    }
  }
}

if (errors.length) {
  console.error(errors.join('\n'));
  process.exit(1);
}
console.log(`i18n dictionaries match: ${referenceKeys.length} keys`);
