import { readdirSync, readFileSync, existsSync, statSync } from 'node:fs';
import { resolve, dirname, join } from 'node:path';

const root = resolve('dist');
const walk = dir => readdirSync(dir, { withFileTypes: true }).flatMap(entry =>
  entry.isDirectory() ? walk(join(dir, entry.name)) : [join(dir, entry.name)]);
const pages = walk(root).filter(file => file.endsWith('.html'));
const failures = [];
let checked = 0;
for (const page of pages) {
  const html = readFileSync(page, 'utf8');
  for (const [, raw] of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    const href = raw.replaceAll('&amp;', '&');
    if (/^(?:https?:|mailto:|tel:|data:|javascript:|\/\/)/.test(href) && !href.startsWith('http://localhost:4321/')) continue;
    const local = href.replace('http://localhost:4321', '');
    const [pathname, fragment] = local.split('#');
    let target = pathname.split('?')[0];
    target = target ? resolve(target.startsWith('/') ? root : dirname(page), '.' + (target.startsWith('/') ? target : '/' + target)) : page;
    if (existsSync(target) && statSync(target).isDirectory()) target = join(target, 'index.html');
    // Astro emits the error route as 404.html with a /404/ canonical URL.
    if (!existsSync(target) && target === join(root, '404')) target = join(root, '404.html');
    checked++;
    if (!existsSync(target)) failures.push(`${page}: missing ${href}`);
    else if (fragment && target.endsWith('.html')) {
      const text = readFileSync(target, 'utf8');
      const id = decodeURIComponent(fragment);
      if (!text.includes(`id="${id}"`)) failures.push(`${page}: missing anchor ${href}`);
    }
  }
}
if (failures.length) { console.error(failures.join('\n')); process.exitCode = 1; }
else console.log(`Checked ${checked} internal links/assets across ${pages.length} HTML pages.`);
