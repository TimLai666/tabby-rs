type SetImmediate = (callback: (...args: any[]) => void, ...args: any[]) => number

const tauriWindow = window as unknown as { setImmediate?: SetImmediate }

tauriWindow.setImmediate ??= ((callback, ...args) => window.setTimeout(callback, 0, ...args)) as SetImmediate
