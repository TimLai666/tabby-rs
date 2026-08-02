export interface SudoPromptMatch {
    username: string|null
}

const MAX_PROMPT_TAIL = 4096

const SUDO_PROMPT_PATTERNS: RegExp[] = [
    /^\[sudo\] password for ([^:]+):\s*$/im,
    /^\[sudo\] Passwort für ([^:]+):\s*$/im,
    /^\[sudo\] Mot de passe de ([^:]+)\s+:\s*$/im,
    /^\[sudo\] [Cc]ontraseña para ([^:]+):\s*$/im,
    /^\[sudo\] [Ss]enha para ([^:]+):\s*$/im,
    /^\[sudo\] [Pp]assword di ([^:]+):\s*$/im,
    /^\[sudo\] ([^\s]+) 的密码[：:]\s*$/im,
    /^\[sudo\] ([^\s]+) 的密碼[：:]\s*$/im,
    /^\[sudo\] ([^\s]+) のパスワード[：:]\s*$/im,
    /^\[sudo\] ([^\s]+) 암호[：:]\s*$/im,
    /^\[sudo\] пароль для ([^:]+):\s*$/im,
    /^\[sudo\] hasło użytkownika ([^:]+):\s*$/im,
    /^\[sudo\] ([^\s]+) için parola:\s*$/im,
    /^\[sudo\] [Hh]eslo pro ([^:]+):\s*$/im,
    /^\[sudo\] lösenord för ([^:]+):\s*$/im,
    /^\[sudo\] adgangskode for ([^:]+):\s*$/im,
    /^\[sudo\] kata sandi untuk ([^:]+):\s*$/im,
    /^\[sudo\] пароль до ([^:]+):\s*$/im,
    /^\[sudo\] lozinka za ([^:]+):\s*$/im,
    /^\[sudo: authenticate\] .+?[：:]\s*$/im,
]

function stripAnsi (value: string): string {
    return value.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '')
}

export class SudoPromptDetector {
    private tail = ''
    private totalCharacters = 0
    private lastMatchEnd = -1
    private lastMatchedPatternIndex = 0

    feed (value: string): SudoPromptMatch|null {
        if (!value) {
            return null
        }

        const clean = stripAnsi(value)
        this.totalCharacters += clean.length
        this.tail = (this.tail + clean).slice(-MAX_PROMPT_TAIL)
        const tailStart = this.totalCharacters - this.tail.length

        for (let offset = 0; offset < SUDO_PROMPT_PATTERNS.length; offset++) {
            const index = (this.lastMatchedPatternIndex + offset) % SUDO_PROMPT_PATTERNS.length
            const match = SUDO_PROMPT_PATTERNS[index].exec(this.tail)
            if (!match) {
                continue
            }

            const absoluteEnd = tailStart + match.index + match[0].length
            if (absoluteEnd <= this.lastMatchEnd) {
                continue
            }

            this.lastMatchEnd = absoluteEnd
            this.lastMatchedPatternIndex = index
            return {
                username: match[1]?.trim() || null,
            }
        }

        return null
    }

    reset (): void {
        this.tail = ''
        this.totalCharacters = 0
        this.lastMatchEnd = -1
        this.lastMatchedPatternIndex = 0
    }
}
