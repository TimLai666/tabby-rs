'use strict'

const emptyKey = Object.freeze({})

module.exports = {
    HK: Object.freeze({ LM: 'HKLM', CU: 'HKCU' }),
    getRegistryKey () {
        return emptyKey
    },
    getRegistryValue () {
        return null
    },
    listRegistrySubkeys () {
        return []
    },
}
