#!/usr/bin/env node

const args = process.argv.slice(2)
const argument = name => {
    const index = args.indexOf(name)
    return index === -1 ? null : args[index + 1]
}

const channel = argument('--channel') || process.env.TABBY_RS_RELEASE_CHANNEL || null
const repository = argument('--repo') || process.env.GITHUB_REPOSITORY || null
const issueList = argument('--issues') || ''
const apiBase = (argument('--api-base') || process.env.TABBY_RS_GITHUB_API_BASE || 'https://api.github.com').replace(/\/$/, '')

if (channel !== 'stable') {
    console.log(`Stable issue gate skipped for ${channel || 'unknown'} release channel.`)
    process.exit(0)
}

if (!/^[^/\s]+\/[^/\s]+$/.test(repository || '')) {
    throw new Error(`invalid GitHub repository: ${repository || '<missing>'}`)
}

const issues = [...new Set(issueList.split(',').map(value => value.trim()).filter(Boolean))]
if (issues.length === 0 || issues.some(value => !/^\d+$/.test(value) || Number(value) < 1)) {
    throw new Error('stable issue gate requires a non-empty comma-separated list of issue numbers')
}

const headers = {
    accept: 'application/vnd.github+json',
    'user-agent': 'tabby-rs-release-gate',
}
if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`

const results = await Promise.all(issues.map(async number => {
    const response = await fetch(`${apiBase}/repos/${repository}/issues/${number}`, { headers })
    if (!response.ok) {
        throw new Error(`GitHub issue #${number} lookup failed with HTTP ${response.status}`)
    }
    const issue = await response.json()
    return { number, state: issue.state, title: issue.title || '' }
}))

const openIssues = results.filter(issue => issue.state !== 'closed')
if (openIssues.length > 0) {
    console.error('Stable release is blocked until all Epic #1 child issues are closed:')
    for (const issue of openIssues) console.error(`- #${issue.number} ${issue.state || 'missing'} ${issue.title}`)
    process.exit(1)
}

console.log(`Stable issue gate passed for ${results.length} Epic #1 child issue(s).`)
