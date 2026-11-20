/**
 * Profile id sent to the native SSH, Telnet and serial managers, which reject an
 * empty id. Quick-connect profiles are never saved and keep the core default id
 * '', so they get an id scoped to their own connection instead.
 */
export function nativeProfileId (profile: { id?: string|null }, connectionId: string): string {
    const id = profile.id?.trim()
    return id ? profile.id! : `unsaved:${connectionId}`
}
