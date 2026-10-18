import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, extname, join, resolve } from 'node:path';

const root = process.cwd();
// Root governance files (READMEs, CONTRIBUTING, SECURITY, AGENTS) plus every
// markdown file under docs/ — all internal links and anchors are verified.
const files = [
  join(root, 'README.md'),
  join(root, 'README.zh-CN.md'),
  join(root, 'CONTRIBUTING.md'),
  join(root, 'CONTRIBUTING.en-US.md'),
  join(root, 'SECURITY.md'),
  join(root, 'AGENTS.md'),
];
const walk = (dir) => {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) walk(path);
    else if (extname(path) === '.md') files.push(path);
  }
};
walk(join(root, 'docs'));

const errors = [];
for (const file of files) {
  const markdown = readFileSync(file, 'utf8');
  for (const match of markdown.matchAll(/!?\[[^\]]*]\(([^)\s]+)(?:\s+"[^"]*")?\)/g)) {
    const target = match[1];
    if (/^(?:https?:|mailto:|#)/.test(target)) continue;
    const [relative, anchor] = target.split('#');
    const destination = resolve(dirname(file), decodeURIComponent(relative));
    if (!existsSync(destination)) {
      errors.push(`${file}: missing ${target}`);
      continue;
    }
    if (anchor && extname(destination) === '.md') {
      const headings =
        readFileSync(destination, 'utf8')
          .match(/^#{1,6}\s+.+$/gm)
          ?.map((line) =>
            line
              .replace(/^#{1,6}\s+/, '')
              .trim()
              .toLowerCase()
              .replace(/[^\p{L}\p{N}\s-]/gu, '')
              .replace(/\s+/g, '-'),
          ) ?? [];
      if (!headings.includes(decodeURIComponent(anchor).toLowerCase())) {
        errors.push(`${file}: missing anchor ${target}`);
      }
    }
  }
}
if (errors.length) {
  console.error(errors.join('\n'));
  process.exit(1);
}
console.log(`Markdown links verified: ${files.length} files`);
