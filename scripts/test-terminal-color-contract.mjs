#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const source = await readFile('tabby-local/src/session.ts', 'utf8')
const environmentBlock = source.match(/let env = mergeEnv\([\s\S]*?\n\s*\},\n\s*substituteEnv\(options\.env\)/)?.[0]

assert(environmentBlock, 'Local session environment merge block is missing')
assert.match(
    environmentBlock,
    /this\.hostApp\.platform === Platform\.macOS \? \{ CLICOLOR: '1' \} : \{\}/,
    'macOS local sessions must enable terminal color output',
)
assert(
    environmentBlock.indexOf('CLICOLOR') < environmentBlock.indexOf('substituteEnv(options.env)'),
    'User-provided environment must be able to override the macOS color default',
)

console.log('Terminal color environment contract passed.')
