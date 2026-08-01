import * as path from 'path'
import * as url from 'url'

import config from '../webpack.plugin.config.mjs'

const __dirname = url.fileURLToPath(new URL('.', import.meta.url))

export default () => config({
    name: 'tauri',
    dirname: __dirname,
    alias: {
        'tabby-core': path.resolve(__dirname, '../tabby-core/src/index.ts'),
    },
})
