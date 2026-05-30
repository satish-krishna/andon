#!/usr/bin/env node
// Capture the two non-page-shaped views the standard release-screenshot
// script doesn't cover:
//
//   docs/images/session-detail.png  -- /sessions/:id  for the first session
//                                      returned by the v2 sessions list.
//   docs/images/files-filtered.png  -- /files with a single repo filter
//                                      pre-applied (via /sessions?repo=...
//                                      which seeds FilterService, then a
//                                      client-side nav click to /files so the
//                                      root-scoped filter sticks).
//
// Prerequisites match capture-release-screenshots.js: andon must be running
// on :8765, the SPA must be built, and `cd scripts && npm install` must have
// run at least once.
//
// Usage (from the scripts/ directory):
//   node capture-extras-screenshots.js

const http = require('http');
const fs = require('fs');
const path = require('path');
const puppeteer = require('puppeteer-core');

const REPO = path.join(__dirname, '..');
const SPA_DIR = path.join(REPO, 'web', 'dist', 'web', 'browser');
const API = 'http://127.0.0.1:8765';
const SERVE_PORT = 8089; // distinct from the release script's 8088
const WIDTH = 1440;
const OUT_DIR = path.join(REPO, 'docs', 'images');

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

async function fetchFirstSessionId() {
  const res = await fetch(API + '/api/v2/sessions?limit=1');
  if (!res.ok) die('GET /api/v2/sessions failed: HTTP ' + res.status);
  const j = await res.json();
  if (!j.sessions || !j.sessions.length) die('no sessions in database');
  return j.sessions[0].session_id;
}


// Same dollar-blur as the release script: blur any element containing text
// that matches a dollar-cost pattern. Repo paths, token counts and session
// IDs are intentionally left intact.
async function blurCosts(page) {
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

// The dashboard scrolls inside an inner container; fullPage misses it. Size
// the viewport to the document's own scrollHeight, capped at 9000. (The
// release script scans every element's scrollHeight, but inner containers
// with overflow-auto + max-height — e.g. the session-detail Timeline — report
// huge scrollHeights that bloat the screenshot. body/document scrollHeight
// is the right measure of actual page extent.)
async function tallestScrollHeight(page) {
  return await page.evaluate(() => {
    const h = Math.max(
      document.documentElement.scrollHeight,
      document.body.scrollHeight,
    );
    return Math.min(h + 48, 9000);
  });
}

(async () => {
  if (!fs.existsSync(path.join(SPA_DIR, 'index.html'))) {
    die('SPA not built — run `cd web; npm run build` (or build-release.ps1) first');
  }
  const browserPath = findBrowser();
  await checkApi();

  const sid = await fetchFirstSessionId();
  console.log('first session: ' + sid);

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

    // Single SPA bootstrap. After this, all navigation goes through clicks
    // on real Angular-routed links — the only mechanism we've verified works
    // end-to-end on the production build. Direct `page.goto` to deep routes
    // and synthetic popstate dispatches both proved unreliable.
    // The SPA uses HashLocationStrategy: every route lives in the URL hash
    // (e.g. http://host/#/sessions/abc). Navigation is therefore driven by
    // setting window.location.hash, which fires `hashchange` — the only event
    // HashLocationStrategy subscribes to. page.goto() with a path, history
    // pushState + popstate, and href="/path" selectors all fail because they
    // address the wrong part of the URL.
    await page.goto(`${base}/`, { waitUntil: 'networkidle0', timeout: 60000 });
    // Wait for the sidebar to render — networkidle0 can fire before Angular
    // commits its first view.
    try {
      await page.waitForSelector('a[href="#/sessions"]', { timeout: 15000 });
    } catch (e) {
      const debug = await page.evaluate(() => ({
        url: window.location.href,
        anchorHrefs: [...document.querySelectorAll('a')].map((a) => a.getAttribute('href')).slice(0, 20),
      }));
      console.error('SIDEBAR NEVER RENDERED. page state:');
      console.error(JSON.stringify(debug, null, 2));
      throw e;
    }
    await sleep(1500);

    async function spaNavigate(routePath) {
      await page.evaluate((p) => {
        // location.hash setter automatically fires hashchange when the value
        // changes. Angular's HashLocationStrategy subscribes to that.
        window.location.hash = p;
      }, routePath);
    }

    async function assertHash(expectedPrefix) {
      const url = await page.url();
      const hashPath = (url.split('#')[1] || '').split('?')[0];
      if (!hashPath.startsWith(expectedPrefix)) {
        die('expected hash path to start with ' + expectedPrefix + ', got ' + hashPath);
      }
    }

    async function captureFullPage(outFile) {
      const h = await tallestScrollHeight(page);
      await page.setViewport({ width: WIDTH, height: h });
      await sleep(1000); // reflow
      await blurCosts(page);
      await sleep(200);
      await page.screenshot({ path: path.join(OUT_DIR, outFile) });
      console.log('  ' + outFile);
    }

    // ---- session-detail.png ------------------------------------------------
    // Direct hash nav to the session-detail route. The route is defined as
    // `sessions/:id` with `loadComponent` — the lazy chunk fetches on first
    // visit, so we wait an extra beat.
    await page.setViewport({ width: WIDTH, height: 1000 });
    await spaNavigate('/sessions/' + encodeURIComponent(sid));
    await sleep(3500);
    await assertHash('/sessions/');
    await captureFullPage('session-detail.png');

    // ---- files-filtered.png ------------------------------------------------
    // Visit /files, then click the FIRST repo chip the page renders. This
    // sidesteps a data trap: /api/repos with no filter window returns the
    // all-time top repo, but the Files page only renders chips for repos
    // active in the current filter window — so a pre-fetched key can yield
    // an empty result. Picking from the page's own list guarantees the
    // filter produces visible data.
    await page.setViewport({ width: WIDTH, height: 1000 });
    await spaNavigate('/files');
    await sleep(3500);
    await assertHash('/files');

    const filtered = await page.evaluate(() => {
      const labels = [...document.querySelectorAll('span.filter-label')];
      const repoLabel = labels.find((el) => el.textContent.trim().toLowerCase().includes('repo'));
      if (!repoLabel) return { ok: false, reason: 'no Repo label (no repos in window?)' };
      const row = repoLabel.parentElement;
      const chip = row && row.querySelector('button.filter-chip');
      if (!chip) return { ok: false, reason: 'no chip in Repo row' };
      chip.click();
      return { ok: true, label: chip.textContent.trim() };
    });
    if (!filtered.ok) die('could not apply repo filter: ' + filtered.reason);
    console.log('  applied repo chip: "' + filtered.label + '"');
    await sleep(3500); // filter re-fetch
    await captureFullPage('files-filtered.png');
  } finally {
    await browser.close();
    server.close();
  }

  console.log('done — review docs/images/session-detail.png and docs/images/files-filtered.png.');
})().catch((e) => die(e.stack || e.message));
