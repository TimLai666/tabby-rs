import { Injectable } from '@angular/core'

import { Logger, LogService, ConfigService, UpdateChannel, UpdateInfo, UpdaterService, PlatformService, TranslateService } from 'tabby-core'
import { ElectronService } from '../services/electron.service'

const UPDATES_URL = 'https://api.github.com/repos/eugeny/tabby/releases/latest'

@Injectable()
export class ElectronUpdaterService extends UpdaterService {
    private logger: Logger
    private downloadPromise: Promise<boolean>|null = null
    private electronUpdaterAvailable = true
    private updateURL: string
    private channel: UpdateChannel = 'stable'

    constructor (
        log: LogService,
        config: ConfigService,
        private translate: TranslateService,
        private platform: PlatformService,
        private electron: ElectronService,
    ) {
        super()
        this.logger = log.create('updater')

        if (process.platform === 'linux' || process.env.PORTABLE_EXECUTABLE_FILE) {
            this.electronUpdaterAvailable = false
            return
        }

        this.electron.ipcRenderer.on('updater:update-available', () => {
            this.logger.info('Update available')
        })

        this.electron.ipcRenderer.on('updater:update-not-available', () => {
            this.logger.info('No updates')
        })

        this.electron.ipcRenderer.on('updater:error', err => {
            this.logger.error(err)
            this.electronUpdaterAvailable = false
        })

        config.ready$.toPromise().then(() => {
            if (config.store.enableAutomaticUpdates && this.electronUpdaterAvailable && !process.env.TABBY_DEV) {
                this.logger.debug('Checking for updates')
                try {
                    this.electron.ipcRenderer.send('updater:check-for-updates')
                } catch (e) {
                    this.electronUpdaterAvailable = false
                    this.logger.info('Electron updater unavailable, falling back', e)
                }
            }
        })
    }

    async check (): Promise<UpdateInfo|null> {
        if (this.electronUpdaterAvailable) {
            return new Promise((resolve, reject) => {
                // eslint-disable-next-line @typescript-eslint/init-declarations, prefer-const
                let cancel
                const onNoUpdate = () => {
                    cancel()
                    resolve(null)
                }
                const onUpdate = (_event, updateInfo) => {
                    cancel()
                    resolve(this.makeUpdateInfo(
                        updateInfo.version,
                        typeof updateInfo.releaseNotes === 'string' ? updateInfo.releaseNotes : '',
                        updateInfo.releaseDate,
                    ))
                }
                const onError = (err) => {
                    cancel()
                    reject(err)
                }
                cancel = () => {
                    this.electron.ipcRenderer.off('updater:error', onError)
                    this.electron.ipcRenderer.off('updater:update-not-available', onNoUpdate)
                    this.electron.ipcRenderer.off('updater:update-available', onUpdate)
                }
                this.electron.ipcRenderer.on('updater:error', onError)
                this.electron.ipcRenderer.on('updater:update-not-available', onNoUpdate)
                this.electron.ipcRenderer.on('updater:update-available', onUpdate)
                try {
                    this.electron.ipcRenderer.send('updater:check-for-updates')
                } catch (e) {
                    this.electronUpdaterAvailable = false
                    this.logger.info('Electron updater unavailable, falling back', e)
                    reject(e)
                }
            })

        } else {
            this.logger.debug('Checking for updates through fallback method.')
            const response = await fetch(UPDATES_URL)
            const data = await response.json()
            const version = data.tag_name.substring(1)
            if (this.electron.app.getVersion() !== version) {
                this.logger.info('Update available')
                this.updateURL = data.html_url
                return this.makeUpdateInfo(version, data.body || '')
            }
            this.logger.info('No updates')
            return null
        }
        return null
    }

    async download (_info: UpdateInfo): Promise<void> {
        if (!this.electronUpdaterAvailable) {
            return
        }

        if (!this.downloadPromise) {
            this.downloadPromise = new Promise<boolean>((resolve, reject) => {
                let cleanup: () => void = () => undefined
                const onDownloaded = () => {
                    cleanup()
                    resolve(true)
                }
                const onError = err => {
                    cleanup()
                    reject(err)
                }
                cleanup = () => {
                    this.electron.ipcRenderer.off('updater:update-downloaded', onDownloaded)
                    this.electron.ipcRenderer.off('updater:error', onError)
                }
                this.electron.ipcRenderer.once('updater:update-downloaded', onDownloaded)
                this.electron.ipcRenderer.once('updater:error', onError)
                try {
                    this.electron.ipcRenderer.send('updater:download-update')
                } catch (error) {
                    cleanup()
                    reject(error)
                }
            })
        }

        try {
            await this.downloadPromise
        } catch (error) {
            this.downloadPromise = null
            throw error
        }
    }

    async install (_info: UpdateInfo): Promise<void> {
        if (!this.electronUpdaterAvailable) {
            await this.electron.shell.openExternal(this.updateURL)
        } else {
            if ((await this.platform.showMessageBox(
                {
                    type: 'warning',
                    message: this.translate.instant('Installing the update will close all tabs and restart Tabby.'),
                    buttons: [
                        this.translate.instant('Update'),
                        this.translate.instant('Cancel'),
                    ],
                    defaultId: 0,
                    cancelId: 1,
                },
            )).response === 0) {
                await this.downloadPromise
                this.electron.ipcRenderer.send('updater:quit-and-install')
            }
        }
    }

    async setChannel (channel: UpdateChannel): Promise<void> {
        this.channel = channel
    }

    async getChannel (): Promise<UpdateChannel> {
        return this.channel
    }

    private makeUpdateInfo (version: string, notes = '', publishedAt = new Date().toISOString()): UpdateInfo {
        return {
            version,
            currentVersion: this.electron.app.getVersion(),
            channel: this.channel,
            publishedAt,
            notes,
            requiresConfigMigration: false,
        }
    }
}
