import { Component } from '@angular/core'
import { ForwardedPortConfig } from '../../../tabby-ssh/src/api/interfaces'
import { TauriSshSession } from './session'

@Component({
    templateUrl: './portForwardingModal.component.pug',
})
export class TauriSshPortForwardingModalComponent {
    session: TauriSshSession

    onForwardAdded (forwarding: ForwardedPortConfig): void {
        void this.session.addPortForward({ ...forwarding }).catch(() => undefined)
    }

    onForwardRemoved (forwarding: ForwardedPortConfig): void {
        void this.session.removePortForward(forwarding).catch(() => undefined)
    }
}
