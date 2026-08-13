import assert from 'node:assert/strict'
import fs from 'node:fs/promises'

const workflow = await fs.readFile(new URL('../.github/workflows/docs.yml', import.meta.url), 'utf8')

assert.match(
    workflow,
    /FIREBASE_SERVICE_ACCOUNT_TABBY_DOCS:\s*\$\{\{\s*secrets\.FIREBASE_SERVICE_ACCOUNT_TABBY_DOCS\s*\}\}/,
    'Docs workflow must expose the Firebase credential through an environment value',
)
assert.match(
    workflow,
    /if:\s*\$\{\{\s*github\.ref\s*==\s*'refs\/heads\/master'\s*&&\s*env\.FIREBASE_SERVICE_ACCOUNT_TABBY_DOCS\s*!=\s*''\s*\}\}/,
    'Docs deployment must be gated on the master branch and a non-empty Firebase credential',
)
assert.match(
    workflow,
    /firebaseServiceAccount:\s*'\$\{\{\s*env\.FIREBASE_SERVICE_ACCOUNT_TABBY_DOCS\s*\}\}'/,
    'Docs deployment must pass the guarded environment value to Firebase',
)

console.log('Docs workflow credential guard contract passed')
