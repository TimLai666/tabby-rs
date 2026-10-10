import { Directive, ElementRef, OnDestroy, OnInit, Self } from '@angular/core'
import { DirectoryUpload, NotificationsService } from 'tabby-core'
import { Subscription } from 'rxjs'
import { DropZoneDirective } from '../../tabby-core/src/directives/dropZone.directive'
import { HostEventMap } from './api/hostBridge'
import { TauriPlatformService } from './services/platform.service'

/** Routes native file drops through the shared SFTP drop-zone output. */
@Directive({ selector: '[dropZone]' })
export class TauriFileDropDirective implements OnInit, OnDestroy {
    private destroyed = false
    private subscription?: Subscription

    constructor (
        private el: ElementRef,
        @Self() private dropZone: DropZoneDirective,
        private platform: TauriPlatformService,
        private notifications: NotificationsService,
    ) { }

    ngOnInit (): void {
        this.subscription = this.platform.fileDropped$.subscribe(event => { void this.onDrop(event) })
    }

    ngOnDestroy (): void {
        this.destroyed = true
        this.subscription?.unsubscribe()
    }

    private async onDrop (event: HostEventMap['desktop:fileDrop']): Promise<void> {
        if (this.isDestroyed() || !event.paths.length) { return }
        const position = this.platform.getFileDropPosition(event)
        const target = document.elementFromPoint(position.x, position.y)
        if (target?.closest('[dropZone]') !== this.el.nativeElement) { return }
        try {
            const transfer = await this.platform.startUploadFromPaths(event.paths, true)
            if (this.isDestroyed()) { this.cancelUploads(transfer) } else { this.dropZone.transfer.emit(transfer) }
        } catch (error) { this.showError(error) }
    }

    private isDestroyed (): boolean { return this.destroyed }

    private cancelUploads (directory: DirectoryUpload): void {
        for (const child of directory.getChildrens()) {
            if (child instanceof DirectoryUpload) { this.cancelUploads(child) } else { child.cancel() }
        }
    }

    private showError (error: any): void {
        if (!this.destroyed) {
            this.notifications.error(typeof error?.details === 'string' ? error.details
                : typeof error?.message === 'string' ? error.message : String(error))
        }
    }
}
