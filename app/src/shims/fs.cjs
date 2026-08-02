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
    statSync: unsupported,
    promises: new Proxy({}, {
        get: () => async () => unsupported(),
    }),
}
