import type { SFTPPanelComponent } from '../components/sftpPanel.component'
import { SFTPContextMenuItemProvider as Provider } from './sftpContextMenu'

/** Extend to add items to the SFTPPanel context menu. */
export interface SFTPContextMenuItemProvider<TPanel = SFTPPanelComponent> extends Provider<TPanel> { weight: number }
// Preserve the original concrete panel default and the shared injection token.
// eslint-disable-next-line @typescript-eslint/no-redeclare
export const SFTPContextMenuItemProvider: abstract new<TPanel = SFTPPanelComponent>() => Provider<TPanel> = Provider
