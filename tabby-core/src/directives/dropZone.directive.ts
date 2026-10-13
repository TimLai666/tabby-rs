import { Directive, Output, ElementRef, EventEmitter, AfterViewInit } from '@angular/core'
import { DirectoryUpload, PlatformService } from '../api/platform'
import './dropZone.directive.scss'

/** @hidden */
@Directive({
    selector: '[dropZone]',
})
export class DropZoneDirective implements AfterViewInit {
    @Output() transfer = new EventEmitter<DirectoryUpload>()
    private dropHint?: HTMLElement

    constructor (
        private el: ElementRef,
        private platform: PlatformService,
    ) { }

    ngAfterViewInit (): void {
        this.el.nativeElement.addEventListener('dragover', () => {
            this.setHovered(true)
        })
        this.el.nativeElement.addEventListener('drop', async (event: DragEvent) => {
            this.setHovered(false)
            this.transfer.emit(await this.platform.startUploadFromDragEvent(event, true))
        })
        this.el.nativeElement.addEventListener('dragleave', () => {
            this.setHovered(false)
        })
    }

    setHovered (hovered: boolean): void {
        if (!hovered) { this.removeHint(); return }
        if (this.dropHint) { return }
        const element = document.createElement('div')
        this.dropHint = element
        element.className = 'drop-zone-hint'
        element.innerHTML = require('./dropZone.directive.pug')
        this.el.nativeElement.appendChild(element)
        setTimeout(() => {
            if (this.dropHint === element) { element.classList.add('visible') }
        })
    }

    private removeHint () {
        const element = this.dropHint
        if (!element) { return }
        delete this.dropHint
        element.classList.remove('visible')
        setTimeout(() => {
            element.remove()
        }, 500)
    }
}
