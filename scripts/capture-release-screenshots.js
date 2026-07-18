#!/usr/bin/env node
// Capture release-notes screenshots of the andon dashboard.
//
// Drives headless Chrome through the running app, blurs every dollar cost,
// and writes one PNG per page to docs/images/release/v<version>/.
//
// Prerequisites:
//   - andon is running        (its API must answer on http://127.0.0.1:8765)
//   - the SPA is built        (`cd web; npm run build` — build-release.ps1 does this)
//   - deps installed          (`cd scripts; npm install`)
//
// Usage (from the scripts/ directory):  node capture-release-screenshots.js
//
// The output PNGs still show real repo paths, token counts and session IDs —
// only cost figures are blurred. Review them before committing.

const http = require('http');
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const REPO = path.join(__dirname, '..');
const SPA_DIR = path.join(REPO, 'web', 'dist', 'web', 'browser');
const API = 'http://127.0.0.1:8765';
const SERVE_PORT = 8088;
const WIDTH = 1440;

// Dashboard pages: sidebar route, sidebar label, output filename.
const PAGES = [
  ['/overview', 'overview', '01-overview'],
  ['/sessions', 'sessions', '02-sessions'],
  ['/files', 'files', '03-files'],
  ['/behaviour', 'behaviour', '04-behaviour'],
  ['/diagnostics', 'diagnostics', '05-diagnostics'],
  ['/settings', 'settings', '06-settings'],
  ['/efficiency', 'efficiency', '07-efficiency'],
  ['/memory', 'memory', '08-memory'],
];

const BROWSER_CANDIDATES = [
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
  'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
  'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
  'C:/Program Files/Microsoft/Edge/Application/msedge.exe',
];

const MIME = {
  '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript',
  '.css': 'text/css', '.json': 'application/json', '.ico': 'image/x-icon',
  '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.woff': 'font/woff',
  '.ttf': 'font/ttf', '.png': 'image/png', '.jpg': 'image/jpeg',
  '.webp': 'image/webp', '.txt': 'text/plain',
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const die = (msg) => {
  console.error('ERROR: ' + msg);
  process.exit(1);
};

function readVersion() {
  const conf = path.join(REPO, 'src-tauri', 'tauri.conf.json');
  const version = JSON.parse(fs.readFileSync(conf, 'utf8')).version;
  if (!version) die('no version field in ' + conf);
  return version;
}

function findBrowser() {
  const found = BROWSER_CANDIDATES.find((p) => fs.existsSync(p));
  if (!found) die('no Chrome or Edge found — install Google Chrome');
  return found;
}

// Serves the built SPA, falling back to index.html for client-side routes.
function startServer() {
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      const urlPath = decodeURIComponent(req.url.split('?')[0]);
      let file = path.join(SPA_DIR, urlPath);
      try {
        if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) {
          file = path.join(SPA_DIR, 'index.html');
        }
        res.writeHead(200, {
          'content-type': MIME[path.extname(file).toLowerCase()] || 'application/octet-stream',
        });
        res.end(fs.readFileSync(file));
      } catch (e) {
        res.writeHead(500);
        res.end(String(e));
      }
    });
    server.listen(SERVE_PORT, '127.0.0.1', () => resolve(server));
  });
}

async function checkApi() {
  try {
    const res = await fetch(API + '/api/health');
    if (!res.ok) throw new Error('HTTP ' + res.status);
  } catch (e) {
    die('andon API not reachable at ' + API + ' — start andon first (' + e.message + ')');
  }
}

(async () => {
  const version = readVersion();
  if (!fs.existsSync(path.join(SPA_DIR, 'index.html'))) {
    die('SPA not built — run `cd web; npm run build` (or build-release.ps1) first');
  }
  const browserPath = findBrowser();
  await checkApi();

  const outDir = path.join(REPO, 'docs', 'images', 'release', 'v' + version);
  fs.mkdirSync(outDir, { recursive: true });
  console.log('version ' + version + '  ->  ' + outDir);

  const server = await startServer();
  const base = 'http://127.0.0.1:' + SERVE_PORT;

  const browser = await puppeteer.launch({
    executablePath: browserPath,
    headless: true,
    args: ['--hide-scrollbars', '--disable-gpu'],
    defaultViewport: { width: WIDTH, height: 1000 },
  });
  try {
    const page = await browser.newPage();
    await page.goto(base + '/overview', { waitUntil: 'networkidle0', timeout: 60000 });
    await sleep(2500);

    for (const [href, label, name] of PAGES) {
      await page.setViewport({ width: WIDTH, height: 1000 });

      // Navigate via the sidebar — same as a user clicking.
      await page.evaluate(
        (h, l) => {
          let a = document.querySelector(`a[href="${h}"]`);
          if (!a) {
            a = [...document.querySelectorAll('a')].find(
              (el) => el.textContent.trim().toLowerCase() === l,
            );
          }
          if (a) a.click();
        },
        href,
        label,
      );
      await sleep(3500); // lazy route + its data fetch settle

      // The layout scrolls inside an inner container, so fullPage misses it;
      // size the viewport to the tallest scrollHeight on the page instead.
      const height = await page.evaluate(() => {
        let max = document.documentElement.scrollHeight;
        for (const el of document.querySelectorAll('*')) {
          if (el.scrollHeight > max) max = el.scrollHeight;
        }
        return Math.min(max + 48, 9000);
      });
      await page.setViewport({ width: WIDTH, height });
      await sleep(1200); // reflow

      // Privacy: blur every element showing a dollar cost. Repo paths,
      // token counts and session IDs are deliberately left intact.
      await page.evaluate(() => {
        const COST = /\$\s?\d[\d,]*\.?\d*/;
        const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
        const hits = [];
        while (walker.nextNode()) {
          if (COST.test(walker.currentNode.textContent)) hits.push(walker.currentNode);
        }
        for (const tn of hits) {
          if (tn.parentElement) tn.parentElement.style.filter = 'blur(10px)';
        }
      });
      await sleep(300);

      await page.screenshot({ path: path.join(outDir, name + '.png') });
      console.log('  ' + name + '.png');
    }
  } finally {
    await browser.close();
    server.close();
  }

  console.log('done — review the PNGs, then embed them in the release notes.');
})().catch((e) => die(e.stack || e.message));
