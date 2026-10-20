// Keep the upstream sound when Vorbis-in-Ogg is supported by this WebView.
const audio = document.createElement('audio')
module.exports = audio.canPlayType('audio/ogg; codecs="vorbis"')
    ? require('../../../tabby-terminal/src/bell.ogg')
    : require('../../assets/bell.wav')
