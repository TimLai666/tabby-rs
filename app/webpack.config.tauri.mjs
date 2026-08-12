import * as fs from 'fs'
import * as path from 'path'
import * as url from 'url'
import wp from 'webpack'
import { AngularWebpackPlugin } from '@ngtools/webpack'
import { createEs2015LinkerPlugin } from '@angular/compiler-cli/linker/babel'

const __dirname = url.fileURLToPath(new URL('.', import.meta.url))
const appNodeModules = path.resolve(__dirname, 'node_modules')
const coreNodeModules = path.resolve(__dirname, '../tabby-core/node_modules')
const localNodeModules = path.resolve(__dirname, '../tabby-local/node_modules')
const terminalNodeModules = path.resolve(__dirname, '../tabby-terminal/node_modules')
const rootNodeModules = path.resolve(__dirname, '../node_modules')
const webNodeModules = path.resolve(__dirname, '../web/node_modules')
const processBrowser = path.resolve(__dirname, 'src/shims/process.cjs')
const fsShim = path.resolve(__dirname, 'src/shims/fs.cjs')
const mzFsShim = path.resolve(__dirname, 'src/shims/mz-fs.ts')
const windowsRegistryShim = path.resolve(__dirname, 'src/shims/windows-native-registry.cjs')

const linkerPlugin = createEs2015LinkerPlugin({
    linkerJitMode: true,
    fileSystem: {
        resolve: path.resolve,
        exists: fs.existsSync,
        dirname: path.dirname,
        relative: path.relative,
        readFile: fs.readFileSync,
    },
})

export default () => ({
    name: 'tabby-tauri-renderer',
    target: 'web',
    entry: {
        'index.ignore': '!!file-loader?name=index.html!pug-html-loader!' + path.resolve(__dirname, 'index.tauri.pug'),
        bundle: path.resolve(__dirname, 'src/entry.tauri.ts'),
    },
    mode: process.env.TABBY_DEV ? 'development' : 'production',
    optimization: {
        minimize: false,
        concatenateModules: false,
    },
    context: __dirname,
    devtool: 'source-map',
    output: {
        path: path.join(__dirname, 'dist-tauri'),
        pathinfo: true,
        filename: '[name].js',
        publicPath: 'auto',
        clean: true,
    },
    resolve: {
        alias: {
            '@ngx-translate/core': path.join(appNodeModules, '@ngx-translate/core'),
            'fs$': fsShim,
            'fs/promises$': fsShim,
            'mz/fs$': mzFsShim,
            'process/browser': processBrowser,
            'process': processBrowser,
            'tabby-core': path.resolve(__dirname, '../tabby-core/src/index.ts'),
            'tabby-local': path.resolve(__dirname, '../tabby-local/src/index.ts'),
            'tabby-settings': path.resolve(__dirname, '../tabby-settings/src/index.ts'),
            'tabby-tauri': path.resolve(__dirname, '../tabby-tauri/src/index.ts'),
            'tabby-terminal': path.resolve(__dirname, '../tabby-terminal/src/index.ts'),
            'windows-native-registry$': windowsRegistryShim,
        },
        modules: [
            'node_modules',
            path.join(__dirname, 'src'),
            appNodeModules,
            coreNodeModules,
            localNodeModules,
            terminalNodeModules,
            rootNodeModules,
            webNodeModules,
            path.join(__dirname, 'assets'),
        ],
        extensions: ['.ts', '.js', '.cjs'],
        mainFields: ['browser', 'module', 'main'],
        fallback: {
            assert: path.join(webNodeModules, 'assert/assert.js'),
            buffer: path.join(webNodeModules, 'buffer/index.js'),
            constants: path.join(webNodeModules, 'constants-browserify/constants.json'),
            crypto: path.join(webNodeModules, 'crypto-browserify/index.js'),
            events: path.join(webNodeModules, 'events/events.js'),
            path: path.join(webNodeModules, 'path-browserify/index.js'),
            process: processBrowser,
            stream: path.join(webNodeModules, 'stream-browserify/index.js'),
            url: path.join(webNodeModules, 'url/url.js'),
            util: path.join(webNodeModules, 'util/util.js'),
            child_process: false,
            dns: false,
            fs: fsShim,
            http: false,
            https: false,
            module: false,
            net: false,
            os: false,
            readline: false,
            tls: false,
            tty: false,
            vm: false,
            zlib: false,
        },
    },
    module: {
        rules: [
            {
                test: /\.(m?)js$/,
                loader: 'babel-loader',
                options: {
                    plugins: [linkerPlugin],
                    compact: false,
                    cacheDirectory: true,
                },
                resolve: {
                    fullySpecified: false,
                },
            },
            {
                test: /\.ts$/,
                use: {
                    loader: '@ngtools/webpack',
                },
            },
            {
                test: /\.pug$/,
                use: [
                    'apply-loader',
                    {
                        loader: 'pug-loader',
                        options: { pretty: true },
                    },
                ],
            },
            { test: /\.scss$/, use: ['@tabby-gang/to-string-loader', 'css-loader', 'sass-loader'], include: /(theme.*|component)\.scss/ },
            { test: /\.scss$/, use: ['style-loader', 'css-loader', 'sass-loader'], exclude: /(theme.*|component)\.scss/ },
            { test: /\.css$/, use: ['@tabby-gang/to-string-loader', 'css-loader'], include: /component\.css/ },
            { test: /\.css$/, use: ['style-loader', 'css-loader'], exclude: /component\.css/ },
            { test: /\.yaml$/, use: ['yaml-loader'] },
            { test: /\.svg$/, use: ['svg-inline-loader'] },
            {
                test: /\.(eot|otf|woff|woff2|ogg)(\?v=[0-9]\.[0-9]\.[0-9])?$/,
                type: 'asset',
            },
            { test: /\.ttf$/, type: 'asset/inline' },
            {
                test: /\.po$/,
                use: [
                    { loader: 'json-loader' },
                    { loader: 'po-gettext-loader' },
                ],
            },
        ],
    },
    plugins: [
        new wp.DefinePlugin({
            global: 'globalThis',
            'process.type': JSON.stringify('renderer'),
            'process.platform': '(window.__TABBY_PLATFORM__ || "browser")',
            'process.arch': '(window.__TABBY_ARCH__ || "unknown")',
            'process.env.TABBY_DEV': JSON.stringify(false),
            'process.env.TABBY_FORCE_ANGULAR_PROD': JSON.stringify(false),
        }),
        new wp.ProvidePlugin({
            Buffer: [path.join(webNodeModules, 'buffer/index.js'), 'Buffer'],
            process: processBrowser,
        }),
        new AngularWebpackPlugin({
            tsconfig: path.resolve(__dirname, 'tsconfig.tauri.json'),
            directTemplateLoading: false,
            jitMode: true,
        }),
    ],
})
