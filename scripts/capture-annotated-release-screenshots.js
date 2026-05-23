#!/usr/bin/env node
// Capture release-notes "partial" screenshots — cropped + (optionally)
// highlighted views of specific page regions. Companion to
// capture-release-screenshots.js, which captures the full pages.
//
// Drives headless Chrome through the running app, blurs every dollar cost,
// finds each requested anchor element, applies an optional outline, and
// writes a cropped PNG of that region (with padding) to
// docs/images/release/v<version>/.
//
// Prerequisites:
//   - andon is running        (its API must answer on http://127.0.0.1:8765)
//   - the SPA is built        (`cd web; npm run build` — build-release.ps1 does this)
//   - deps installed          (`cd scripts; npm install`)
//
// Usage (from the scripts/ directory):  node capture-annotated-release-screenshots.js
//
// Edit the CAPTURES list below per release. Each entry produces one PNG
// named `<output>.png` in the version folder.

const http = require('http');
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const REPO = path.join(__dirname, '..');
const SPA_DIR = path.join(REPO, 'web', 'dist', 'web', 'browser');
const API = 'http://127.0.0.1:8765';
const SERVE_PORT = 8089; // distinct from the full-page script's 8088
const WIDTH = 1440;

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

// ============================================================================
// Captures
// ----------------------------------------------------------------------------
// Each entry produces one cropped PNG.
//
//   output     — filename without extension; <version>/<output>.png
//   route      — sidebar route to navigate to (e.g. '/overview')
//   anchor     — how to find the target region:
//                { by: 'selector', value: <CSS selector> }
//                { by: 'text', value: <visible text>, climb: <CSS selector> }
//                  ('climb' = element.closest(selector), defaults to 'section')
//   highlight  — optional CSS selector OR { by, value, climb } anchor to
//                outline with a 3px accent-yellow ring before the screenshot
//   padding    — pixels of whitespace around the clip rect (default 24)
//
// Tuned to the v0.5.0 release; update per release.
// ============================================================================
const CAPTURES = [
  // rc.2: Overview tape — month-over-month cost bars.
  {
    output: '10-overview-tape',
    route: '/overview',
    anchor: { by: 'text', value: 'tape' }, // matches " · The tape"
    padding: 12,
  },
  // rc.4: Budget bar on the Overview cost tile.
  {
    output: '11-overview-budget-cost-tile',
    route: '/overview',
    anchor: { by: 'text', value: 'Budget', climb: 'section' },
    padding: 12,
  },
  // rc.4: Monthly budget card on Settings.
  {
    output: '12-settings-monthly-budget',
    route: '/settings',
    anchor: { by: 'text', value: 'Monthly budget', climb: 'section' },
    padding: 12,
  },
  // rc.3 / rc.1: Behaviour page — model mix.
  {
    output: '13-behaviour-model-mix',
    route: '/behaviour',
    anchor: { by: 'text', value: 'Model mix', climb: 'section' },
    padding: 12,
  },
  // rc.6: Sessions table — the new Totals row and code/docs/other split.
  // The totals row + segmented bar live at the bottom of the table;
  // we crop the whole sessions panel to show them in context.
  {
    output: '14-sessions-totals-and-split',
    route: '/sessions',
    anchor: { by: 'selector', value: 'table' },
    padding: 12,
  },
  // #28: Efficiency page — Experimental banner.
  {
    output: '20-efficiency-experimental-banner',
    route: '/efficiency',
    anchor: { by: 'selector', value: '.border-warn\\/40' },
    padding: 8,
  },
  // #28: Efficiency page — explainer blurb (with dynamic "your numbers" line).
  {
    output: '21-efficiency-explainer',
    route: '/efficiency',
    anchor: { by: 'text', value: 'How prompt caching saves you money', climb: 'section' },
    padding: 12,
  },
  // #28: Efficiency page — KPI tile strip.
  {
    output: '22-efficiency-kpi-strip',
    route: '/efficiency',
    anchor: { by: 'selector', value: '.grid.grid-cols-3' },
    padding: 12,
  },
  // #28: Efficiency page — Model cost-efficiency table.
  {
    output: '23-efficiency-model-table',
    route: '/efficiency',
    anchor: { by: 'text', value: 'Model cost-efficiency', climb: 'section' },
    padding: 12,
  },
];

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

// Click the sidebar link for `route`, wait for lazy-loaded content + data.
async function navigateVia(page, route) {
  await page.evaluate((r) => {
    const a = document.querySelector(`a[href="${r}"]`);
    if (a) a.click();
  }, route);
  await sleep(3500); // lazy chunk + first data fetch
}

// Compute the bounding rect of the anchor element in the page's coordinate
// space (CSS pixels, top-left origin), accounting for whatever scrolling
// container holds it.
async function rectOf(page, anchor) {
  return page.evaluate((a) => {
    function find() {
      if (a.by === 'selector') return document.querySelector(a.value);
      if (a.by === 'text') {
        const climb = a.climb || 'section';
        // Find the deepest element whose own (own-text-only, trimmed) content
        // matches; then climb to the nearest enclosing structural ancestor.
        const all = Array.from(document.querySelectorAll('*'));
        const hit = all.find((el) => {
          if (el.children.length === 0) return false; // leaf-only matchers below
          const own = Array.from(el.childNodes)
            .filter((n) => n.nodeType === 3)
            .map((n) => n.textContent.trim())
            .join(' ');
          return own.toLowerCase().includes(a.value.toLowerCase());
        }) || all.find((el) => el.textContent.trim().toLowerCase().includes(a.value.toLowerCase()));
        return hit ? hit.closest(climb) || hit : null;
      }
      return null;
    }
    const el = find();
    if (!el) return null;
    el.scrollIntoView({ block: 'start', inline: 'nearest', behavior: 'instant' });
    const r = el.getBoundingClientRect();
    return { x: Math.max(0, r.left), y: Math.max(0, r.top), width: r.width, height: r.height };
  }, anchor);
}

async function applyOutline(page, anchor) {
  await page.evaluate((a) => {
    const el = (a.by === 'selector')
      ? document.querySelector(a.value)
      : Array.from(document.querySelectorAll('*')).find(
          (el) => el.textContent.trim().toLowerCase().includes(a.value.toLowerCase()),
        );
    if (el) {
      el.style.outline = '3px solid #facc15';
      el.style.outlineOffset = '4px';
      el.style.borderRadius = el.style.borderRadius || '4px';
    }
  }, anchor);
}

async function blurDollars(page) {
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
  console.log('captures: ' + CAPTURES.length);

  const server = await startServer();
  const base = 'http://127.0.0.1:' + SERVE_PORT;

  const browser = await puppeteer.launch({
    executablePath: browserPath,
    headless: true,
    args: ['--hide-scrollbars', '--disable-gpu'],
    defaultViewport: { width: WIDTH, height: 1200 },
  });
  try {
    const page = await browser.newPage();
    await page.goto(base + '/overview', { waitUntil: 'networkidle0', timeout: 60000 });
    await sleep(2500);

    let lastRoute = '/overview';
    for (const cap of CAPTURES) {
      if (cap.route !== lastRoute) {
        await navigateVia(page, cap.route);
        lastRoute = cap.route;
      }

      // Make the viewport tall enough for content below the fold.
      const height = await page.evaluate(() => {
        let max = document.documentElement.scrollHeight;
        for (const el of document.querySelectorAll('*')) {
          if (el.scrollHeight > max) max = el.scrollHeight;
        }
        return Math.min(max + 48, 9000);
      });
      await page.setViewport({ width: WIDTH, height });
      await sleep(800); // reflow

      await blurDollars(page);
      if (cap.highlight) await applyOutline(page, cap.highlight);
      await sleep(200);

      const rect = await rectOf(page, cap.anchor);
      if (!rect) {
        console.error('  ' + cap.output + '.png — anchor not found, skipping (' + JSON.stringify(cap.anchor) + ')');
        continue;
      }
      const pad = cap.padding ?? 24;
      const clip = {
        x: Math.max(0, Math.floor(rect.x - pad)),
        y: Math.max(0, Math.floor(rect.y - pad)),
        width: Math.min(WIDTH, Math.ceil(rect.width + 2 * pad)),
        height: Math.ceil(rect.height + 2 * pad),
      };
      // Clamp to viewport so puppeteer doesn't fail on regions that extend
      // past the bottom of a short page.
      if (clip.y + clip.height > height) clip.height = height - clip.y;

      await page.screenshot({ path: path.join(outDir, cap.output + '.png'), clip });
      console.log('  ' + cap.output + '.png  (' + clip.width + 'x' + clip.height + ')');

      // Remove the outline before the next iteration so it doesn't bleed.
      if (cap.highlight) {
        await page.evaluate(() => {
          for (const el of document.querySelectorAll('*')) {
            if (el.style && el.style.outline === '3px solid rgb(250, 204, 21)') {
              el.style.outline = '';
              el.style.outlineOffset = '';
            }
          }
        });
      }
    }
  } finally {
    await browser.close();
    server.close();
  }

  console.log('done — review the PNGs, then embed them in the release notes.');
})().catch((e) => die(e.stack || e.message));
