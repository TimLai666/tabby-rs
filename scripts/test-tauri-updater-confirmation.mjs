import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/updater.service.ts'), 'utf8')
const updateCommand = fs.readFileSync(path.join(root, 'src-tauri/src/commands/update.rs'), 'utf8')
const updateService = fs.readFileSync(path.join(root, 'src-tauri/src/update/service.rs'), 'utf8')

assert.match(source, /PlatformService, TranslateService/)
assert.match(source, /await this\.platform\.showMessageBox\(\{[\s\S]*buttons:/)
assert.match(source, /return confirmation\.response === 0/)
assert.match(source, /override async update \(info: UpdateInfo\): Promise<void> \{[\s\S]*await this\.confirmInstall\(\)[\s\S]*await this\.download\(info\)[\s\S]*await this\.installConfirmed\(info\)/)
assert.match(source, /async install \(info: UpdateInfo\): Promise<void> \{[\s\S]*await this\.confirmInstall\(\)[\s\S]*await this\.installConfirmed\(info\)/)
const updateMethod = source.match(/override async update \(info: UpdateInfo\): Promise<void> \{([\s\S]*?)\n    \}/)?.[1]
assert.ok(updateMethod, 'Tauri updater update method must be present')
assert.ok(updateMethod.indexOf('await this.confirmInstall()') < updateMethod.indexOf('await this.download(info)'), 'update confirmation must precede download')
assert.ok(updateMethod.indexOf('await this.download(info)') < updateMethod.indexOf('await this.installConfirmed(info)'), 'download must precede install')
assert.match(updateCommand, /if configured_endpoint\(&request\.channel\)\.is_none\(\) \|\| configured_public_key\(\)\.is_none\(\)/)
assert.match(updateCommand, /configured_public_key\(\)[\s\S]*return Err\(AppError::Io\([\s\S]*let paths/)
assert.match(updateCommand, /emit_state\(&app, &state\);\n    check_update\(&app, &state\)\.await\?/)
assert.match(updateService, /\.configure_client\(configure_updater_client\)/)
assert.match(updateService, /client\.redirect\(reqwest::redirect::Policy::none\(\)\)/)

const installMethod = updateCommand.match(/pub async fn update_install\([\s\S]*?\n\}\n\n#\[tauri::command\]/)?.[0]
assert.ok(installMethod, 'Tauri updater install method must be present')
const journalWriteIndex = installMethod.indexOf('write_pending_update_journal_with_binary(')
assert.ok(installMethod.indexOf('create_backup(') < journalWriteIndex, 'backup must be created before the rollback journal')
assert.ok(journalWriteIndex < installMethod.indexOf('ready.update.install('), 'rollback journal must be persisted before installation')
assert.match(
    installMethod,
    /let paths = StoragePaths::from_app_paths\(state\.paths\(\)\)[\s\S]*?if !state[\s\S]*?install_is_current\(install_generation\)[\s\S]*?return abort_cancelled_install\(&state, &paths, install_generation\)/,
    'a cancellation observed before backup creation must clear persisted rollback state',
)
assert.match(installMethod, /Err\(_\) => \{[\s\S]*?restore_ready\(ready\)[\s\S]*?update backup could not be created/)
const journalFailureIndex = installMethod.indexOf('update journal could not be persisted')
assert.ok(journalFailureIndex > journalWriteIndex, 'journal failure path must remain after journal write')
assert.match(installMethod.slice(journalWriteIndex, journalFailureIndex), /restore_ready\(ready\)/)
assert.match(installMethod, /rollback_result\.is_err\(\)[\s\S]*?std::process::exit\(1\)/)
assert.match(installMethod, /if current_state\.update_channel == UpdateChannel::Stable \{[\s\S]*?remember_stable_backup\(persisted, &backup\.backup_id\)/)

console.log('Tauri updater confirmation contract passed')
