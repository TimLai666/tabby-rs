import 'zone.js'
import 'core-js/proposals/reflect-metadata'
import 'rxjs'

import './global.scss'
import './toastr.scss'

// Importing before @angular/*
import { findPlugins, initModuleLookup, loadPlugins } from './plugins'
import { ipcRenderer } from 'electron'

import { BootstrapData, PluginInfo } from '../../tabby-core/src/api/mainProcess'
import { bootstrapTabby } from './bootstrap'

location.hash = ''

;(process as any).enablePromiseAPI = true

if (process.platform === 'win32' && !('HOME' in process.env)) {
    process.env.HOME = `${process.env.HOMEDRIVE}${process.env.HOMEPATH}`
}

const debug = !!process.env.TABBY_DEV && !process.env.TABBY_FORCE_ANGULAR_PROD
if (debug) {
    console.warn('Running in debug mode')
}

async function loadAndBootstrap (
    bootstrapData: BootstrapData,
    plugins: PluginInfo[],
    safeMode = false,
): Promise<void> {
    const selectedPlugins = safeMode ? plugins.filter(x => x.isBuiltin) : plugins
    const pluginModules = await loadPlugins(selectedPlugins, (current, total) => {
        const progressBar = document.querySelector('.progress .bar') as HTMLElement | null
        if (progressBar) {
            progressBar.style.width = `${100 * current / total}%`
        }
    })
    await bootstrapTabby(bootstrapData, pluginModules, { debug })
}

ipcRenderer.once('start', async (_$event, bootstrapData: BootstrapData) => {
    console.log('Window bootstrap data:', bootstrapData)
    initModuleLookup(bootstrapData.userPluginsPath)

    let plugins = await findPlugins()
    bootstrapData.installedPlugins = plugins
    if (bootstrapData.config.pluginBlacklist) {
        plugins = plugins.filter(x => !bootstrapData.config.pluginBlacklist.includes(x.name))
    }
    plugins = plugins.filter(x => x.name !== 'web')

    console.log('Starting with plugins:', plugins)
    try {
        await loadAndBootstrap(bootstrapData, plugins)
    } catch (error) {
        console.error('Angular bootstrapping error:', error)
        console.warn('Trying safe mode')
        window['safeModeReason'] = error
        try {
            await loadAndBootstrap(bootstrapData, plugins, true)
        } catch (error2) {
            console.error('Bootstrap failed:', error2)
        }
    }
})

ipcRenderer.send('ready')
