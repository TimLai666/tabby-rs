import type { MenuItemOptions } from 'tabby-core'
import type { SFTPFile } from './sftp'

/** Host-neutral runtime token shared by original and native SFTP panels. */
export abstract class SFTPContextMenuItemProvider<TPanel> {
    weight = 0
    abstract getItems (item: SFTPFile, panel: TPanel): Promise<MenuItemOptions[]>
}
