class ThrowingPlugin {
    constructor () {
        throw new Error('fixture constructor failed')
    }
}

module.exports = { default: new ThrowingPlugin() }
