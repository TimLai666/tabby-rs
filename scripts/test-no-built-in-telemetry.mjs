import assert from 'node:assert/strict'

import { findTelemetryViolations, scanProductionFiles } from './ci/assert-no-built-in-telemetry.mjs'

assert.deepEqual(
    findTelemetryViolations('const dsn = "https://example.sentry.io/123"', 'fixture.js'),
    [{ rule: 'sentry-sdk-or-endpoint', path: 'fixture.js' }],
)
assert.deepEqual(
    findTelemetryViolations('import mixpanel from "mixpanel-browser"', 'fixture.js'),
    [{ rule: 'mixpanel-sdk-or-endpoint', path: 'fixture.js' }],
)
assert.deepEqual(findTelemetryViolations('class Sentry { consume () {} }', 'zmodem.ts'), [])
assert.deepEqual(scanProductionFiles(), [])

console.log('Built-in telemetry audit fixtures passed')
