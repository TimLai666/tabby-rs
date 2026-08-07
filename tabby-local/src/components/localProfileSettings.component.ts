/* eslint-disable @typescript-eslint/explicit-module-boundary-types */
import { Component, Inject, OnInit, Optional } from '@angular/core'
import { FullyDefined, PlatformService, ProfileSettingsComponent, VaultService } from 'tabby-core'

import { LocalProfile, UACService } from '../api'
import { LocalProfilesService } from '../profiles'

const SUDO_SECRET_TYPE = 'sudo:password'

/** @hidden */
@Component({
    templateUrl: './localProfileSettings.component.pug',
})
export class LocalProfileSettingsComponent implements ProfileSettingsComponent<LocalProfile, LocalProfilesService>, OnInit {
    profile: FullyDefined<LocalProfile>
    sudoPasswordDraft = ''
    sudoPasswordSaved = false

    constructor (
        @Optional() @Inject(UACService) public uac: UACService|undefined,
        public vault: VaultService,
        private platform: PlatformService,
    ) { }

    async ngOnInit (): Promise<void> {
        await this.refreshSudoPasswordState()
    }

    async pickWorkingDirectory (): Promise<void> {
        const cwd = await this.platform.pickDirectory()
        if (!cwd) {
            return
        }
        this.profile.options.cwd = cwd
    }

    async saveSudoPassword (): Promise<void> {
        if (!this.vault.isEnabled()) {
            throw new Error('Enable Vault before saving an automatic sudo password')
        }
        if (!this.sudoPasswordDraft) {
            return
        }
        await this.vault.addSecret({
            type: SUDO_SECRET_TYPE,
            key: { profileId: this.profile.id },
            value: this.sudoPasswordDraft,
        })
        this.sudoPasswordDraft = ''
        this.profile.options.sudoSecretRef = this.sudoSecretRef
        this.profile.options.autoSudoPassword = true
        this.sudoPasswordSaved = true
    }

    async removeSudoPassword (): Promise<void> {
        if (this.vault.isEnabled()) {
            await this.vault.removeSecret(SUDO_SECRET_TYPE, { profileId: this.profile.id })
        }
        this.sudoPasswordDraft = ''
        this.sudoPasswordSaved = false
        this.profile.options.autoSudoPassword = false
        this.profile.options.sudoSecretRef = null
    }

    private async refreshSudoPasswordState (): Promise<void> {
        if (!this.vault.isEnabled()) {
            this.sudoPasswordSaved = false
            this.profile.options.autoSudoPassword = false
            this.profile.options.sudoSecretRef = null
            return
        }
        const secret = await this.vault.getSecret(SUDO_SECRET_TYPE, { profileId: this.profile.id })
        this.sudoPasswordSaved = !!secret
        if (secret) {
            this.profile.options.sudoSecretRef = this.sudoSecretRef
        } else {
            this.profile.options.autoSudoPassword = false
            this.profile.options.sudoSecretRef = null
        }
    }

    private get sudoSecretRef (): string {
        return `vault:profile:${this.profile.id}`
    }
}
