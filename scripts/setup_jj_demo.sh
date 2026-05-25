#!/usr/bin/env bash
set -euo pipefail

# Setup a jj demo project: a Node.js CLI app built up across bookmarks.
# Usage: ./scripts/setup_jj_demo.sh [target-dir]
#   target-dir defaults to /tmp/jj-hello-demo

DEMO_DIR="${1:-/tmp/jj-hello-demo}"

if [ -d "$DEMO_DIR" ]; then
  echo "Removing existing demo at $DEMO_DIR"
  rm -rf "$DEMO_DIR"
fi

mkdir -p "$DEMO_DIR"
cd "$DEMO_DIR"

# ── 1. Initialize jj repo ──────────────────────────────────────────────
jj git init
jj config set --repo user.name "Demo User"
jj config set --repo user.email "demo@example.com"

# ── 2. Bookmark: main — "hello world" ──────────────────────────────────
cat > package.json <<'EOF'
{
  "name": "hello-cli",
  "version": "1.0.0",
  "description": "A friendly greeting CLI",
  "main": "index.js",
  "bin": {
    "hello": "./index.js"
  },
  "scripts": {
    "start": "node index.js",
    "test": "node test.js"
  }
}
EOF

cat > index.js <<'JSEOF'
#!/usr/bin/env node

console.log("hello world!");
JSEOF
chmod +x index.js

cat > .gitignore <<'EOF'
node_modules/
EOF

jj commit -m "feat: initial hello world CLI"
jj bookmark create main -r @-

# ── 3. Bookmark: feature/random-planet — replace "world" with a planet ─
jj new main -m "wip: add random planet greeting"
jj bookmark create feature/random-planet

cat > index.js <<'JSEOF'
#!/usr/bin/env node

const planets = [
  "Mercury", "Venus", "Mars", "Jupiter",
  "Saturn", "Uranus", "Neptune", "Pluto"
];

function randomPlanet() {
  return planets[Math.floor(Math.random() * planets.length)];
}

const planet = randomPlanet();
console.log(`hello ${planet}!`);
JSEOF

jj commit -m "feat: greet a random planet instead of world"

# ── 4. Bookmark: feature/personalized — add name argument ──────────────
jj new feature/random-planet -m "wip: add personalized greeting"
jj bookmark create feature/personalized

cat > index.js <<'JSEOF'
#!/usr/bin/env node

const planets = [
  "Mercury", "Venus", "Mars", "Jupiter",
  "Saturn", "Uranus", "Neptune", "Pluto"
];

function randomPlanet() {
  return planets[Math.floor(Math.random() * planets.length)];
}

const name = process.argv[2];
const planet = randomPlanet();

if (name) {
  console.log(`hello ${name} from ${planet}!`);
} else {
  console.log(`hello ${planet}!`);
}
JSEOF

jj commit -m "feat: personalized greeting with name argument"

# ── 5. Bookmark: docs — add README documentation ──────────────────────
jj new main -m "wip: add project documentation"
jj bookmark create docs

cat > README.md <<'MDEOF'
# hello-cli

A friendly greeting CLI built with Node.js.

## Installation

```bash
npm link
```

## Usage

```bash
# Basic greeting (random planet)
node index.js

# Personalized greeting
node index.js Alice
```

## Output Examples

```
hello Jupiter!
hello Alice from Saturn!
```
MDEOF

jj commit -m "docs: add README with usage instructions"

# ── 6. Bookmark: test — add test file ─────────────────────────────────
jj new feature/personalized -m "wip: add tests"
jj bookmark create test

cat > test.js <<'JSEOF'
const { execSync } = require("child_process");

let passed = 0;
let failed = 0;

function assert(description, actual, pattern) {
  if (pattern.test(actual.trim())) {
    console.log(`  ✓ ${description}`);
    passed++;
  } else {
    console.log(`  ✗ ${description}`);
    console.log(`    expected to match: ${pattern}`);
    console.log(`    got: ${actual.trim()}`);
    failed++;
  }
}

console.log("Running tests...\n");

// Test 1: no argument — should greet a planet
const out1 = execSync("node index.js").toString();
assert(
  "no args prints hello <planet>!",
  out1,
  /^hello (Mercury|Venus|Mars|Jupiter|Saturn|Uranus|Neptune|Pluto)!$/
);

// Test 2: with name — should include name and planet
const out2 = execSync("node index.js Alice").toString();
assert(
  "with name prints hello <name> from <planet>!",
  out2,
  /^hello Alice from (Mercury|Venus|Mars|Jupiter|Saturn|Uranus|Neptune|Pluto)!$/
);

// Test 3: with a different name
const out3 = execSync("node index.js Bob").toString();
assert(
  "with another name prints hello <name> from <planet>!",
  out3,
  /^hello Bob from (Mercury|Venus|Mars|Jupiter|Saturn|Uranus|Neptune|Pluto)!$/
);

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
JSEOF

jj commit -m "test: add CLI output tests"

# ── 7. Go back to main ────────────────────────────────────────────────
jj new main

echo ""
echo "═══════════════════════════════════════════════════════"
echo "  Demo repo ready at: $DEMO_DIR"
echo "═══════════════════════════════════════════════════════"
echo ""
echo "Bookmarks created:"
jj bookmark list
echo ""
echo "Log:"
jj log --no-pager
echo ""
echo "Try:  cd $DEMO_DIR && jj log"
