'use strict'

const processShim = {
    nextTick (callback, ...args) {
        queueMicrotask(() => callback(...args))
    },
    title: 'browser',
    browser: true,
    env: {},
    argv: [],
    version: '',
    versions: {},
    platform: 'browser',
    arch: 'unknown',
    cwd () {
        return '/'
    },
    chdir () {
        throw new Error('process.chdir is not supported in the Tauri renderer')
    },
    umask () {
        return 0
    },
    on () {},
    addListener () {},
    once () {},
    off () {},
    removeListener () {},
    removeAllListeners () {},
    emit () {
        return false
    },
}

module.exports = processShim
