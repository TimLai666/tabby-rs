import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import yaml from 'js-yaml'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const packageJson = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'))
const parity = yaml.load(fs.readFileSync(path.join(root, 'parity/features.yaml'), 'utf8'))
const scripts = packageJson.scripts ?? {}
const checks = new Set()

for (const feature of parity.features ?? []) {
    for (const check of feature.tests?.automated ?? []) {
        checks.add(check)
    }
}

const missing = [...checks]
    .filter(check => typeof scripts[`test:${check}`] !== 'string')
    .sort()

assert.deepEqual(missing, [], `parity automated checks without yarn entry points: ${missing.join(', ')}`)
console.log(`Parity automated check links passed: ${checks.size} checks`)
