import { Component, Input, ViewContainerRef, ViewChild, ComponentFactoryResolver, ComponentRef } from '@angular/core'
import { SettingsTabProvider } from '../api'

/** @hidden */
@Component({
    selector: 'settings-tab-body',
    template: '<ng-template #placeholder></ng-template><div class="alert alert-danger" *ngIf="errorMessage">Unable to load this settings page: {{errorMessage}}</div>',
    styles: [`
        :host {
            display: block;
            padding-bottom: 20px;
            max-width: 600px;
        }
    `],
})
export class SettingsTabBodyComponent {
    @Input() provider: SettingsTabProvider
    @ViewChild('placeholder', { read: ViewContainerRef }) placeholder: ViewContainerRef
    component: ComponentRef<unknown>
    errorMessage: string|null = null

    constructor (private componentFactoryResolver: ComponentFactoryResolver) { }

    ngAfterViewInit (): void {
        // run after the change detection finishes
        setImmediate(() => {
            try {
                this.component = this.placeholder.createComponent(
                    this.componentFactoryResolver.resolveComponentFactory(
                        this.provider.getComponentType(),
                    ),
                )
            } catch (error) {
                this.errorMessage = error instanceof Error ? error.message : String(error)
            }
        })
    }
}
