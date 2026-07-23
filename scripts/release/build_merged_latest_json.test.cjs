#!/usr/bin/env node

const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const scriptPath = path.join(__dirname, 'build_merged_latest_json.cjs');

test('builds a legacy manifest for Windows x64 and macOS ARM64 only', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-latest-'));
  const assetsDir = path.join(root, 'assets');
  const notesFile = path.join(root, 'notes.md');
  const outputFile = path.join(root, 'latest.json');
  fs.mkdirSync(assetsDir);

  const supportedAssets = [
    'Cockpit.Tools_1.2.3_aarch64.app.tar.gz',
    'Cockpit.Tools_1.2.3_x64_en-US.msi',
    'Cockpit.Tools_1.2.3_x64-setup.exe',
  ];
  for (const asset of supportedAssets) {
    fs.writeFileSync(path.join(assetsDir, asset), 'artifact');
    fs.writeFileSync(path.join(assetsDir, `${asset}.sig`), `signature-${asset}`);
  }

  // Extra unsupported assets must not leak into the fork's updater manifest.
  fs.writeFileSync(path.join(assetsDir, 'Cockpit.Tools_1.2.3_x64.app.tar.gz'), 'intel');
  fs.writeFileSync(path.join(assetsDir, 'Cockpit.Tools_1.2.3_amd64.AppImage'), 'linux');
  fs.writeFileSync(notesFile, 'Release notes');

  try {
    execFileSync(
      process.execPath,
      [
        scriptPath,
        '--version',
        '1.2.3',
        '--repo',
        'lee3423434234-max/cockpit-tools',
        '--assets-dir',
        assetsDir,
        '--notes-file',
        notesFile,
        '--published-at',
        '2026-07-23T00:00:00Z',
        '--output',
        outputFile,
      ],
      { stdio: 'pipe' },
    );

    const latest = JSON.parse(fs.readFileSync(outputFile, 'utf8'));
    assert.deepEqual(Object.keys(latest.platforms).sort(), [
      'darwin-aarch64',
      'darwin-aarch64-app',
      'windows-x86_64',
      'windows-x86_64-msi',
      'windows-x86_64-nsis',
    ]);
    assert.match(latest.platforms['darwin-aarch64'].url, /_aarch64\.app\.tar\.gz$/);
    assert.match(latest.platforms['windows-x86_64-msi'].url, /_x64_en-US\.msi$/);
    assert.match(latest.platforms['windows-x86_64-nsis'].url, /_x64-setup\.exe$/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
