import { Injectable } from '@angular/core'
import { NgbModal } from '@ng-bootstrap/ng-bootstrap'
import { AsyncSubject, lastValueFrom, Observable, Subject } from 'rxjs'
import {
    StoredVault,
    UnlockVaultModalComponent,
    Vault,
    VaultSecret,
    VaultSecretKey,
} from 'tabby-core'

import {
    HostBridge,
    VaultMutationResult,
    VaultSecretData,
    VaultSecretSelector,
    VaultSnapshot,
} from '../api/hostBridge'

interface UnlockInput {
    passphrase: string
    rememberForSeconds: number
}

@Injectable({ providedIn: 'root' })
export class TauriVaultService {
    get ready$ (): Observable<boolean> { return this.ready }
    get contentChanged$ (): Observable<void> { return this.contentChanged }

    store: StoredVault|null = null

    private ready = new AsyncSubject<boolean>()
    private readyCompleted = false
    private contentChanged = new Subject<void>()
    private open = false
    private expirationTimer: ReturnType<typeof setTimeout>|null = null
    private promptPromise: Promise<UnlockInput>|null = null
    private storeFingerprint: string|null = null

    constructor (
        private bridge: HostBridge,
        private ngbModal: NgbModal,
    ) { }

    async setEnabled (enabled: boolean, passphrase?: string): Promise<void> {
        await this.waitUntilReady()
        if (!enabled) {
            await this.bridge.invoke('vault.setEnabled', { enabled: false })
            this.clearOpenState()
            this.store = null
            this.storeFingerprint = null
            this.contentChanged.next()
            return
        }
        if (this.store) {
            if (!this.open) {
                await this.decrypt(this.store, passphrase)
            }
            return
        }
        const rememberForSeconds = this.rememberSecondsFromPreference()
        const actualPassphrase = passphrase ?? (await this.promptUnlock()).passphrase
        const mutation = await this.bridge.invoke('vault.setEnabled', {
            enabled: true,
            passphrase: actualPassphrase,
            rememberForSeconds,
        })
        if (!mutation) {
            throw new Error('Vault creation did not return an encrypted store')
        }
        this.markOpen(rememberForSeconds)
        this.applyMutation(mutation)
    }

    isOpen (): boolean {
        return this.open
    }

    forgetPassphrase (): void {
        this.clearOpenState()
        void this.bridge.invoke('vault.lock', {}).catch(() => null)
    }

    async decrypt (storage: StoredVault, passphrase?: string): Promise<Vault> {
        const unlock = passphrase
            ? {
                passphrase,
                rememberForSeconds: this.rememberSecondsFromPreference(),
            }
            : await this.promptUnlock()
        await this.bridge.invoke('vault.unlock', {
            stored: storage,
            passphrase: unlock.passphrase,
            rememberForSeconds: unlock.rememberForSeconds,
        })
        this.markOpen(unlock.rememberForSeconds)
        return this.snapshot()
    }

    async load (passphrase?: string): Promise<Vault|null> {
        await this.waitUntilReady()
        if (!this.store) {
            return null
        }
        if (!this.open || passphrase) {
            return this.decrypt(this.store, passphrase)
        }
        try {
            return await this.snapshot()
        } catch {
            this.clearOpenState()
            return this.decrypt(this.store, passphrase)
        }
    }

    async encrypt (vault: Vault, passphrase?: string): Promise<StoredVault|null> {
        await this.waitUntilReady()
        if (passphrase) {
            const mutation = await this.bridge.invoke('vault.replace', {
                vault: this.toSnapshot(vault),
                passphrase,
                rememberForSeconds: this.rememberSecondsFromPreference(),
            })
            this.markOpen(this.rememberSecondsFromPreference())
            this.store = mutation.stored
            this.storeFingerprint = fingerprint(mutation.stored)
            return mutation.stored
        }

        if (this.open) {
            const current = await this.snapshot()
            if (sameSecrets(current.secrets, vault.secrets)) {
                const mutation = await this.bridge.invoke('vault.setConfig', { config: vault.config })
                this.store = mutation.stored
                this.storeFingerprint = fingerprint(mutation.stored)
                return mutation.stored
            }
        }

        const unlock = await this.promptUnlock()
        const mutation = await this.bridge.invoke('vault.replace', {
            vault: this.toSnapshot(vault),
            passphrase: unlock.passphrase,
            rememberForSeconds: unlock.rememberForSeconds,
        })
        this.markOpen(unlock.rememberForSeconds)
        this.store = mutation.stored
        this.storeFingerprint = fingerprint(mutation.stored)
        return mutation.stored
    }

    async save (vault: Vault, passphrase?: string): Promise<void> {
        const stored = await this.encrypt(vault, passphrase)
        if (!stored) {
            throw new Error('Vault encryption returned no store')
        }
        this.store = stored
        this.storeFingerprint = fingerprint(stored)
        this.contentChanged.next()
    }

    async getPassphrase (): Promise<string> {
        return (await this.promptUnlock()).passphrase
    }

    async getSecret (type: string, key: VaultSecretKey): Promise<VaultSecret|null> {
        await this.ensureUnlocked()
        const selector = this.selector(type, key)
        const value = await this.bridge.invoke('vault.getSecret', { selector })
        return value === null ? null : { type, key, value }
    }

    async addSecret (secret: VaultSecret): Promise<void> {
        await this.ensureUnlocked()
        this.applyMutation(await this.bridge.invoke('vault.putSecret', {
            secret: this.secretData(secret),
        }))
    }

    async updateSecret (secret: VaultSecret, update: VaultSecret): Promise<void> {
        await this.ensureUnlocked()
        this.applyMutation(await this.bridge.invoke('vault.updateSecret', {
            selector: this.selector(secret.type, secret.key),
            secret: this.secretData(update),
        }))
    }

    async removeSecret (type: string, key: VaultSecretKey): Promise<void> {
        await this.ensureUnlocked()
        this.applyMutation(await this.bridge.invoke('vault.removeSecret', {
            selector: this.selector(type, key),
        }))
    }

    setStore (store: StoredVault|null|undefined): void {
        const next = store ?? null
        const nextFingerprint = next ? fingerprint(next) : null
        const changedExternally = this.storeFingerprint !== null
            && nextFingerprint !== this.storeFingerprint
        this.store = next
        this.storeFingerprint = nextFingerprint
        if (changedExternally || !next) {
            this.forgetPassphrase()
        }
        if (!this.readyCompleted) {
            this.readyCompleted = true
            this.ready.next(true)
            this.ready.complete()
        }
    }

    isEnabled (): boolean {
        return !!this.store
    }

    private async ensureUnlocked (): Promise<void> {
        await this.waitUntilReady()
        if (!this.store) {
            throw new Error('Vault is not configured')
        }
        if (!this.open) {
            await this.decrypt(this.store)
            return
        }
        const status = await this.bridge.invoke('vault.status', {})
        if (!status.unlocked) {
            this.clearOpenState()
            await this.decrypt(this.store)
        }
    }

    private async snapshot (): Promise<Vault> {
        const snapshot = await this.bridge.invoke('vault.snapshot', {})
        return {
            config: snapshot.config,
            secrets: snapshot.secrets.map(secret => ({
                type: secret.type,
                key: secret.key,
                value: secret.value,
            })),
        }
    }

    private toSnapshot (vault: Vault): VaultSnapshot {
        return {
            config: vault.config,
            secrets: vault.secrets.map(secret => this.secretData(secret)),
        }
    }

    private selector (type: string, key: VaultSecretKey): VaultSecretSelector {
        return {
            type,
            key: key as Record<string, unknown>,
        }
    }

    private secretData (secret: VaultSecret): VaultSecretData {
        return {
            type: secret.type,
            key: secret.key as Record<string, unknown>,
            value: secret.value,
        }
    }

    private applyMutation (mutation: VaultMutationResult): void {
        this.store = mutation.stored
        this.storeFingerprint = fingerprint(mutation.stored)
        this.open = true
        this.contentChanged.next()
    }

    private async promptUnlock (): Promise<UnlockInput> {
        if (!this.promptPromise) {
            this.promptPromise = (async () => {
                const modal = this.ngbModal.open(UnlockVaultModalComponent)
                const result = await modal.result.catch(() => null)
                if (!result) {
                    throw new Error('Vault unlock cancelled')
                }
                return {
                    passphrase: result.passphrase as string,
                    rememberForSeconds: Math.max(1, Number(result.rememberFor) * 60),
                }
            })().finally(() => {
                this.promptPromise = null
            })
        }
        return this.promptPromise
    }

    private markOpen (rememberForSeconds: number): void {
        this.clearTimer()
        this.open = true
        this.expirationTimer = setTimeout(() => {
            this.open = false
            void this.bridge.invoke('vault.lock', {}).catch(() => null)
        }, Math.max(1, rememberForSeconds) * 1000)
    }

    private clearOpenState (): void {
        this.open = false
        this.clearTimer()
    }

    private clearTimer (): void {
        if (this.expirationTimer) {
            clearTimeout(this.expirationTimer)
            this.expirationTimer = null
        }
    }

    private rememberSecondsFromPreference (): number {
        const minutes = Number.parseInt(window.localStorage.vaultRememberPassphraseFor ?? '1', 10)
        return Math.max(1, Number.isFinite(minutes) ? minutes : 1) * 60
    }

    private async waitUntilReady (): Promise<void> {
        await lastValueFrom(this.ready$)
    }
}

function fingerprint (stored: StoredVault): string {
    return `${stored.version}:${stored.keySalt}:${stored.iv}:${stored.contents}`
}

function sameSecrets (left: VaultSecret[], right: VaultSecret[]): boolean {
    return JSON.stringify(left) === JSON.stringify(right)
}
