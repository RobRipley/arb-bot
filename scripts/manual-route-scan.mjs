#!/usr/bin/env node
import { runLocalManualScan } from './local-route-scanner.mjs';

function option(name, fallback) {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : Number(process.argv[index + 1]);
}

const full = process.argv.includes('--full');
const maxWorkItems = full ? 100_000 : option('--max-work-items', 25);
const report = await runLocalManualScan({
  maxRouteLegs: option('--max-route-legs', 3),
  maxWorkItems,
  concurrency: option('--concurrency', 4),
});
console.log(JSON.stringify(report, null, 2));
