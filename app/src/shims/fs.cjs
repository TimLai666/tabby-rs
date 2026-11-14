'use strict'

function unsupported () {
    const error = new Error('Direct renderer filesystem access is unavailable in Tabby RS')
    error.code = 'TABBY_RS_FS_UNAVAILABLE'
    throw error
}

module.exports = {
    existsSync: () => true,
    readFileSync: unsupported,
    writeFileSync: unsupported,
    // The renderer cannot inspect the filesystem; keep the permissive answer
    // existsSync already gives so working-directory checks do not drop paths.
    // Asynchronous checks go through the native host (see mz-fs.ts).
    statSync: () => ({ isDirectory: () => true, isFile: () => false }),
    promises: new Proxy({}, {
        get: () => async () => unsupported(),
    }),
}
