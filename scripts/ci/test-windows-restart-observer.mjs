import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { startRestartProcessObserver } from './restart-process-observer.mjs';
import { proveRestartOrigin } from './restart-process-origin.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OBSERVER_PS1 = path.join(HERE, 'observe-windows-restart.ps1');
const TIMEOUT_MS = 90_000;
const USAGE = `Windows native restart-origin fixture

Usage:
  node test-windows-restart-observer.mjs --output FRESH_ABSOLUTE_DIRECTORY --powershell ABSOLUTE_POWERSHELL_EXE
  node test-windows-restart-observer.mjs --help

On Windows, this starts the native process-start observer and owned held PowerShell
fixtures, records CIM identities and native event history, proves two owned origins,
rejects a same-executable unrelated candidate, and verifies native observer timeout.
All fixture files and result.json are written only inside the fresh output directory.
On other platforms, a valid invocation reports SKIP without starting native processes.`;

function parseArgs (argv) {
    if (argv.length === 1 && argv[0] === '--help') return { help: true };
    if (argv.length !== 4 || argv[0] !== '--output' || argv[2] !== '--powershell') return { error: 'missing or invalid arguments' };
    return { output: argv[1], powerShell: argv[3] };
}

function encodedCommand (script) {
    return Buffer.from(script, 'utf16le').toString('base64');
}

function psLiteral (value) {
    return `'${String(value).replaceAll("'", "''")}'`;
}

function delay (ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

function startProcess (executable, args, label) {
    const child = spawn(executable, args, { windowsHide: true, shell: false, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    let spawnError = null;
    const exited = new Promise(resolve => {
        child.once('exit', (code, signal) => resolve({ label, code, signal }));
    });
    child.stdout.on('data', bytes => { stdout = (stdout + bytes.toString('utf8')).slice(-16000); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes.toString('utf8')).slice(-16000); });
    const closed = new Promise(resolve => {
        child.once('error', error => { spawnError = error; });
        child.once('close', (code, signal) => resolve({ label, code, signal, stdout, stderr, spawnError: spawnError?.message ?? null }));
    });
    return { child, exited, closed, label, get stdout () { return stdout; }, get stderr () { return stderr; } };
}

async function closeWithin (entry, timeoutMs, label = entry.label) {
    let timeout;
    try {
        const outcome = await Promise.race([
            entry.closed,
            new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error(`${label} did not exit before timeout`)), timeoutMs); }),
        ]);
        clearTimeout(timeout);
        if (outcome.spawnError) throw new Error(`${label} failed to spawn: ${outcome.spawnError}`);
        return outcome;
    } finally {
        clearTimeout(timeout);
    }
}

async function exitWithin (entry, timeoutMs, label = entry.label) {
    let timeout;
    try {
        return await Promise.race([
            entry.exited,
            entry.closed.then(outcome => {
                if (outcome.spawnError) throw new Error(`${label} failed to spawn: ${outcome.spawnError}`);
                return { label, code: outcome.code, signal: outcome.signal };
            }),
            new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error(`${label} did not exit before timeout`)), timeoutMs); }),
        ]);
    } finally {
        clearTimeout(timeout);
    }
}

async function runPowerShell (powerShell, script, label, timeoutMs = 10000) {
    const entry = startProcess(powerShell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', encodedCommand(script)], label);
    let outcome;
    try {
        outcome = await closeWithin(entry, timeoutMs);
    } catch (error) {
        if (error.message === `${label} did not exit before timeout`) {
            // This is the exact helper started above; do not target a PID or process tree.
            try { entry.child.kill(); } catch { }
            await Promise.race([entry.closed, delay(3000)]);
        }
        throw error;
    }
    if (outcome.code !== 0) throw new Error(`${label} exited ${outcome.code}: ${outcome.stderr || outcome.stdout}`);
    return outcome.stdout.trim();
}

function queryIdentityScript (pid) {
    return `$ErrorActionPreference = 'Stop'\n` +
        `$row = Get-CimInstance -ClassName Win32_Process -Filter 'ProcessId = ${pid}' -ErrorAction Stop\n` +
        `if (-not $row) { [Console]::Out.WriteLine('null'); exit 0 }\n` +
        `$identity = @{ pid = [int]$row.ProcessId; parentPid = [int]$row.ParentProcessId; creationTimeUTC = $row.CreationDate.ToUniversalTime().ToString('o') }\n` +
        `[Console]::Out.WriteLine((ConvertTo-Json -InputObject $identity -Compress))\n`;
}

async function queryIdentity (powerShell, pid, timeoutMs = 10000) {
    const stdout = await runPowerShell(powerShell, queryIdentityScript(pid), `query-CIM-${pid}`, timeoutMs);
    const record = JSON.parse(stdout);
    if (record === null) return null;
    if (!Number.isSafeInteger(record.pid) || !Number.isSafeInteger(record.parentPid) || typeof record.creationTimeUTC !== 'string') {
        throw new Error(`CIM returned an invalid process row for PID ${pid}`);
    }
    return record;
}

function heldPowerShellScript (metadataPath, stopPath) {
    return `$ErrorActionPreference = 'Stop'\n` +
        `$metadataPath = ${psLiteral(metadataPath)}\n` +
        `$stopPath = ${psLiteral(stopPath)}\n` +
        `$self = Get-CimInstance -ClassName Win32_Process -Filter (\"ProcessId = $PID\") -ErrorAction Stop\n` +
        `if (-not $self) { throw 'CIM did not return this fixture process' }\n` +
        `$identity = @{ pid = [int]$self.ProcessId; parentPid = [int]$self.ParentProcessId; creationTimeUTC = $self.CreationDate.ToUniversalTime().ToString('o') }\n` +
        `Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public static class FixtureNativeBirth{[DllImport("kernel32.dll",SetLastError=true)]public static extern bool GetProcessTimes(IntPtr handle,out long created,out long exited,out long kernel,out long user);}' -ErrorAction Stop\n` +
        `$retained = [Diagnostics.Process]::GetCurrentProcess()\n` +
        `try {\n` +
        `    [long]$created=0;[long]$exited=0;[long]$kernel=0;[long]$user=0\n` +
        `    if (-not [FixtureNativeBirth]::GetProcessTimes($retained.Handle,[ref]$created,[ref]$exited,[ref]$kernel,[ref]$user)) { throw 'Retained-handle GetProcessTimes failed' }\n` +
        `    $identity.nativeCreationFileTime=$created.ToString([Globalization.CultureInfo]::InvariantCulture)\n` +
        `    $identity.nativeCreationUTC=[DateTime]::FromFileTimeUtc($created).ToString('o')\n` +
        `    $identity.nativeObservedUTC=[DateTime]::UtcNow.ToString('o')\n` +
        `    $identity.cimCreationFileTime=$self.CreationDate.ToUniversalTime().ToFileTimeUtc().ToString([Globalization.CultureInfo]::InvariantCulture)\n` +
        `} finally { $retained.Dispose() }\n` +
        `[IO.File]::WriteAllText($metadataPath, (ConvertTo-Json -InputObject $identity -Compress), [Text.UTF8Encoding]::new($false))\n` +
        `$until = [DateTime]::UtcNow.AddMinutes(2)\n` +
        `while (-not (Test-Path -LiteralPath $stopPath -PathType Leaf)) {\n` +
        `    if ([DateTime]::UtcNow -ge $until) { throw 'fixture stop deadline expired' }\n` +
        `    Start-Sleep -Milliseconds 40\n` +
        `}\n`;
}

async function writeScript (outputDir, filename, contents) {
    const absolute = path.join(outputDir, filename);
    writeFileSync(absolute, contents, { flag: 'wx', encoding: 'utf8' });
    return absolute;
}

async function waitForJson (filename, deadline, label) {
    while (Date.now() < deadline) {
        if (existsSync(filename)) {
            try {
                return JSON.parse(readFileSync(filename, 'utf8'));
            } catch (error) {
                if (!(error instanceof SyntaxError)) throw error;
            }
        }
        await delay(40);
    }
    throw new Error(`${label} metadata did not appear before its deadline`);
}

async function startHeldFixture ({ outputDir, powerShell, name, metadataName, stopName, held, nodeChildren }) {
    const metadataPath = path.join(outputDir, metadataName);
    const stopPath = path.join(outputDir, stopName);
    const script = heldPowerShellScript(metadataPath, stopPath);
    const scriptPath = await writeScript(outputDir, `${name}.ps1`, script);
    const entry = startProcess(powerShell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', encodedCommand(script)], name);
    nodeChildren.push(entry);
    const record = { name, metadataPath, stopPath, scriptPath, entry, identity: null };
    held.push(record);
    record.identity = await waitForJson(metadataPath, Date.now() + 15000, `${name} process`);
    if (record.identity.pid !== entry.child.pid) throw new Error(`${name} metadata PID does not match its direct child`);
    const actual = await queryIdentity(powerShell, record.identity.pid);
    if (!sameIdentity(record.identity, actual) || record.identity.parentPid !== process.pid) {
        throw new Error(`${name} PowerShell metadata differs from its exact CIM identity or expected Node parent`);
    }
    if (entry.child.exitCode !== null) throw new Error(`${name} exited before fixture validation`);
    return record;
}

function sameIdentity (a, b) {
    return !!a && !!b && a.pid === b.pid && a.parentPid === b.parentPid && a.creationTimeUTC === b.creationTimeUTC;
}

async function waitForTraceEvents (observer, pids, deadline) {
    let currentTrace;
    while (Date.now() < deadline) {
        currentTrace = observer.trace();
        const observed = new Set(currentTrace.events.map(event => event.pid));
        if (pids.every(pid => observed.has(pid))) return currentTrace;
        await delay(50);
    }
    throw new Error(`native event history did not include required PIDs: ${pids.join(', ')}`);
}

function markCase (cases, name, expected, actual, evidence = {}) {
    const passed = expected === actual;
    cases.push({ name, expected, actual, status: passed ? 'PASS' : 'FAIL', ...evidence });
    if (!passed) throw new Error(`${name}: expected ${expected}, received ${actual}`);
}

function writeStopFile (record) {
    if (!record.stopPath || existsSync(record.stopPath)) return;
    writeFileSync(record.stopPath, `${randomUUID()}\n`, { flag: 'wx', encoding: 'utf8' });
}

async function waitForIdentityGone (powerShell, identity, deadline) {
    while (Date.now() < deadline) {
        const current = await queryIdentity(powerShell, identity.pid, Math.min(5000, Math.max(1000, deadline - Date.now())));
        if (!sameIdentity(identity, current)) return true;
        await delay(80);
    }
    return false;
}

async function runDeadlineObserverCase ({ outputDir, powerShell, deadline, nodeChildren }) {
    const nonce = randomUUID();
    const paths = {
        events: path.join(outputDir, 'deadline-events.jsonl'),
        ready: path.join(outputDir, 'deadline-ready.json'),
        stop: path.join(outputDir, 'deadline-stop'),
    };
    if (Object.values(paths).some(existsSync)) throw new Error('deadline observer evidence paths are not fresh');
    const entry = startProcess(powerShell, [
        '-NoLogo', '-NoProfile', '-NonInteractive', '-File', OBSERVER_PS1,
        '-EventsFile', paths.events, '-ReadyFile', paths.ready, '-StopFile', paths.stop,
        '-Nonce', nonce, '-MaximumSeconds', '1',
    ], 'one-second-native-deadline-observer');
    nodeChildren.push(entry);
    const result = await closeWithin(entry, Math.min(12000, Math.max(1000, deadline - Date.now())));
    let records = [];
    if (existsSync(paths.events)) {
        records = readFileSync(paths.events, 'utf8').split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line));
    }
    const errorRecord = records.find(record => record.kind === 'error');
    const cleanup = entry.child.pid ? await queryIdentity(powerShell, entry.child.pid) === null : false;
    return {
        expectedExitCode: 1,
        exitCode: result.code,
        signal: result.signal,
        errorRecord: errorRecord ?? null,
        records,
        paths,
        pid: entry.child.pid,
        reaped: cleanup,
        passed: result.code === 1 && !!errorRecord && /deadline expired/i.test(errorRecord.message ?? '') && cleanup,
    };
}

async function runFixture ({ outputDir, powerShell }) {
    const startedAt = new Date().toISOString();
    const deadline = Date.now() + TIMEOUT_MS;
    const cases = [];
    const nodeChildren = [];
    const held = [];
    const result = {
        status: 'FAIL',
        platform: process.platform,
        startedAt,
        outputDir,
        powerShell,
        cases,
        cleanup: { held: [], nodeChildren: [], observer: null, deadlineObserver: null },
        guiActions: 0,
        networkAccess: false,
        error: null,
    };
    let observer = null;
    let traceAtEnd = null;
    let direct;
    let unrelatedSeed;
    let relauncher;
    let replacement;
    let deadlineResult;
    let outputCreated = false;
    try {
        if (existsSync(outputDir)) throw new Error('output directory must be fresh and must not already exist');
        mkdirSync(outputDir, { recursive: false });
        outputCreated = true;
        const nodeSeed = await queryIdentity(powerShell, process.pid, Math.min(10000, deadline - Date.now()));
        if (!nodeSeed || nodeSeed.pid !== process.pid) throw new Error('could not capture this Node process exact CIM identity');

        unrelatedSeed = await startHeldFixture({
            outputDir, powerShell, name: 'unrelated-held-seed', metadataName: 'unrelated-seed-cim.json',
            stopName: 'stop-unrelated-seed', held, nodeChildren,
        });
        observer = await startRestartProcessObserver({ powerShell, directory: outputDir, deadline });

        const directTriggeredUtc = new Date().toISOString();
        direct = await startHeldFixture({
            outputDir, powerShell, name: 'direct-owned-candidate', metadataName: 'direct-candidate-cim.json',
            stopName: 'stop-direct-candidate', held, nodeChildren,
        });
        if (direct.identity.parentPid !== nodeSeed.pid) throw new Error('direct candidate does not name the exact Node seed as its CIM parent');
        const directTrace = await waitForTraceEvents(observer, [direct.identity.pid], deadline);
        const directProof = proveRestartOrigin({ candidate: direct.identity, seeds: [nodeSeed], trace: directTrace, triggeredUtc: directTriggeredUtc });
        markCase(cases, 'direct Node seed to held PowerShell candidate', true, directProof.owned, {
            triggeredUtc: directTriggeredUtc,
            seed: nodeSeed,
            candidate: direct.identity,
            proof: directProof,
            trace: directTrace,
        });

        const chainTriggeredUtc = new Date().toISOString();
        const candidateMetaPath = path.join(outputDir, 'relaunch-candidate-cim.json');
        const candidateStopPath = path.join(outputDir, 'stop-relaunch-candidate');
        const candidateScript = heldPowerShellScript(candidateMetaPath, candidateStopPath);
        const candidateScriptPath = await writeScript(outputDir, 'relaunch-candidate.ps1', candidateScript);
        replacement = {
            name: 'relaunch-candidate', metadataPath: candidateMetaPath, stopPath: candidateStopPath,
            scriptPath: candidateScriptPath, entry: null, identity: null,
        };
        held.push(replacement);
        const candidateEncoded = encodedCommand(candidateScript);
        const middleMetaPath = path.join(outputDir, 'intermediate-cim.json');
        const middleStopPath = path.join(outputDir, 'stop-intermediate');
        const allowExitPath = path.join(outputDir, 'allow-intermediate-exit');
        const launchInfoPath = path.join(outputDir, 'relaunch-launch-info.json');
        const middleScript = `$ErrorActionPreference = 'Stop'\n` +
            `$metadataPath = ${psLiteral(middleMetaPath)}\n` +
            `$stopPath = ${psLiteral(middleStopPath)}\n` +
            `$candidateMetadataPath = ${psLiteral(candidateMetaPath)}\n` +
            `$candidateStopPath = ${psLiteral(candidateStopPath)}\n` +
            `$allowExitPath = ${psLiteral(allowExitPath)}\n` +
            `$launchInfoPath = ${psLiteral(launchInfoPath)}\n` +
            `$powerShellPath = ${psLiteral(powerShell)}\n` +
            `$candidateEncoded = ${psLiteral(candidateEncoded)}\n` +
            `$self = Get-CimInstance -ClassName Win32_Process -Filter (\"ProcessId = $PID\") -ErrorAction Stop\n` +
            `if (-not $self) { throw 'CIM did not return this intermediate process' }\n` +
            `$identity = @{ pid = [int]$self.ProcessId; parentPid = [int]$self.ParentProcessId; creationTimeUTC = $self.CreationDate.ToUniversalTime().ToString('o') }\n` +
            `[IO.File]::WriteAllText($metadataPath, (ConvertTo-Json -InputObject $identity -Compress), [Text.UTF8Encoding]::new($false))\n` +
            `$startInfo = [Diagnostics.ProcessStartInfo]::new()\n` +
            `$startInfo.FileName = $powerShellPath\n` +
            `$startInfo.Arguments = '-NoLogo -NoProfile -NonInteractive -EncodedCommand ' + $candidateEncoded\n` +
            `$startInfo.UseShellExecute = $false\n` +
            `$startInfo.CreateNoWindow = $true\n` +
            `if (Test-Path -LiteralPath $stopPath -PathType Leaf) { exit 20 }\n` +
            `$candidateProcess = [Diagnostics.Process]::Start($startInfo)\n` +
            `if (-not $candidateProcess) { throw 'ProcessStartInfo did not create a candidate' }\n` +
            `$launchUntil = [DateTime]::UtcNow.AddSeconds(8)\n` +
            `$launchRow = $null\n` +
            `while (-not $launchRow) {\n` +
            `    $launchRow = Get-CimInstance -ClassName Win32_Process -Filter (\"ProcessId = $($candidateProcess.Id)\") -ErrorAction Stop\n` +
            `    if ($launchRow) {\n` +
            `        if ([int]$launchRow.ParentProcessId -ne $PID) { throw 'candidate CIM parent is not the intermediate process' }\n` +
            `        $launchIdentity = @{ pid = [int]$launchRow.ProcessId; parentPid = [int]$launchRow.ParentProcessId; creationTimeUTC = $launchRow.CreationDate.ToUniversalTime().ToString('o') }\n` +
            `        [IO.File]::WriteAllText($launchInfoPath, (ConvertTo-Json -InputObject $launchIdentity -Compress), [Text.UTF8Encoding]::new($false))\n` +
            `        break\n` +
            `    }\n` +
            `    if ([DateTime]::UtcNow -ge $launchUntil) { throw 'candidate CIM identity did not appear' }\n` +
            `    Start-Sleep -Milliseconds 20\n` +
            `}\n` +
            `$until = [DateTime]::UtcNow.AddSeconds(20)\n` +
            `while (-not (Test-Path -LiteralPath $candidateMetadataPath -PathType Leaf)) {\n` +
            `    if (Test-Path -LiteralPath $stopPath -PathType Leaf) {\n` +
            `        if (-not (Test-Path -LiteralPath $candidateStopPath)) { [IO.File]::WriteAllText($candidateStopPath, 'stop') }\n` +
            `        exit 20\n` +
            `    }\n` +
            `    if ([DateTime]::UtcNow -ge $until) { throw 'relaunch candidate metadata timed out' }\n` +
            `    Start-Sleep -Milliseconds 40\n` +
            `}\n` +
            `$candidate = Get-Content -LiteralPath $candidateMetadataPath -Raw | ConvertFrom-Json\n` +
            `if ([int]$candidate.pid -ne $candidateProcess.Id -or [int]$candidate.parentPid -ne $PID) { throw 'candidate metadata does not identify the launched child' }\n` +
            `$launched = Get-Content -LiteralPath $launchInfoPath -Raw | ConvertFrom-Json\n` +
            `if ([int]$launched.pid -ne [int]$candidate.pid -or $launched.creationTimeUTC -ne $candidate.creationTimeUTC) { throw 'candidate launch and self identities disagree' }\n` +
            `while (-not (Test-Path -LiteralPath $allowExitPath -PathType Leaf)) {\n` +
            `    if (Test-Path -LiteralPath $stopPath -PathType Leaf) {\n` +
            `        if (-not (Test-Path -LiteralPath $candidateStopPath -PathType Leaf)) { [IO.File]::WriteAllText($candidateStopPath, 'stop') }\n` +
            `        exit 20\n` +
            `    }\n` +
            `    if ([DateTime]::UtcNow -ge $until) { throw 'intermediate exit handshake timed out' }\n` +
            `    Start-Sleep -Milliseconds 40\n` +
            `}\n` +
            `$candidateProcess.Dispose()\n`;
        const middleScriptPath = await writeScript(outputDir, 'intermediate-launcher.ps1', middleScript);
        relauncher = startProcess(powerShell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', encodedCommand(middleScript)], 'intermediate-launcher');
        nodeChildren.push(relauncher);
        const middleHeld = { name: 'intermediate-launcher', metadataPath: middleMetaPath, stopPath: middleStopPath, scriptPath: middleScriptPath, entry: relauncher, identity: null };
        held.push(middleHeld);
        middleHeld.identity = await waitForJson(middleMetaPath, Math.min(deadline, Date.now() + 15000), 'intermediate PowerShell process');
        if (middleHeld.identity.pid !== relauncher.child.pid || middleHeld.identity.parentPid !== process.pid) throw new Error('intermediate metadata does not match its exact Node child');
        const actualMiddle = await queryIdentity(powerShell, middleHeld.identity.pid);
        if (!sameIdentity(middleHeld.identity, actualMiddle)) throw new Error('intermediate metadata differs from its CIM identity');

        const launchInfo = await waitForJson(launchInfoPath, Math.min(deadline, Date.now() + 25000), 'relaunch candidate launch identity');
        const candidateIdentity = await waitForJson(candidateMetaPath, Math.min(deadline, Date.now() + 10000), 'relaunch candidate');
        if (!sameIdentity(launchInfo, candidateIdentity)) throw new Error('intermediate launcher and candidate metadata disagree');
        replacement.identity = candidateIdentity;
        const actualCandidate = await queryIdentity(powerShell, candidateIdentity.pid);
        if (!sameIdentity(candidateIdentity, actualCandidate) || candidateIdentity.parentPid !== middleHeld.identity.pid) {
            throw new Error('relaunch candidate does not match its exact post-start CIM identity/parent');
        }
        writeFileSync(allowExitPath, `${randomUUID()}\n`, { flag: 'wx', encoding: 'utf8' });
        // The .NET-launched candidate can keep inherited stdout/stderr pipe handles
        // open after the middle process exits, delaying Node's `close` event.
        const middleExit = await exitWithin(relauncher, Math.min(12000, Math.max(1000, deadline - Date.now())));
        middleHeld.exit = middleExit;
        if (!await waitForIdentityGone(powerShell, middleHeld.identity, Date.now() + 5000)) {
            throw new Error('intermediate PowerShell process remained alive after its natural exit');
        }
        const middleAfterExit = await queryIdentity(powerShell, middleHeld.identity.pid);
        if (sameIdentity(middleHeld.identity, middleAfterExit)) throw new Error('intermediate PowerShell process remained alive after its natural exit');
        const candidateAfterMiddleExit = await queryIdentity(powerShell, candidateIdentity.pid);
        if (!sameIdentity(candidateIdentity, candidateAfterMiddleExit)) {
            throw new Error('relaunch candidate did not remain alive after its intermediate process exited');
        }

        const chainTrace = await waitForTraceEvents(observer, [middleHeld.identity.pid, candidateIdentity.pid], deadline);
        const chainProof = proveRestartOrigin({ candidate: candidateIdentity, seeds: [nodeSeed], trace: chainTrace, triggeredUtc: chainTriggeredUtc });
        markCase(cases, 'Node seed through naturally exited PowerShell relauncher', true, chainProof.owned, {
            triggeredUtc: chainTriggeredUtc,
            seed: nodeSeed,
            intermediate: middleHeld.identity,
            intermediateExit: middleExit,
            candidate: candidateIdentity,
            candidateAfterMiddleExit,
            proof: chainProof,
            trace: chainTrace,
        });

        const unrelatedProof = proveRestartOrigin({
            candidate: candidateIdentity,
            seeds: [unrelatedSeed.identity],
            trace: chainTrace,
            triggeredUtc: chainTriggeredUtc,
        });
        markCase(cases, 'same-executable candidate rejected against unrelated held seed', false, unrelatedProof.owned, {
            triggeredUtc: chainTriggeredUtc,
            seed: unrelatedSeed.identity,
            candidate: candidateIdentity,
            proof: unrelatedProof,
            trace: chainTrace,
            guiActions: 0,
        });

        traceAtEnd = observer.trace();
        deadlineResult = await runDeadlineObserverCase({ outputDir, powerShell, deadline, nodeChildren });
        cases.push({
            name: 'native observer exits with error after one-second deadline without stop file',
            expected: true,
            actual: deadlineResult.passed,
            status: deadlineResult.passed ? 'PASS' : 'FAIL',
            evidence: deadlineResult,
        });
        if (!deadlineResult.passed) throw new Error('one-second native observer deadline case did not produce exit 1, an error record, and reaping');
    } catch (error) {
        result.error = { message: error?.message ?? String(error), stack: error?.stack ?? null };
    } finally {
        for (const record of held) {
            try { writeStopFile(record); } catch (error) { result.cleanup.held.push({ name: record.name, stopWritten: false, error: String(error) }); }
        }

        // The middle launcher is a direct Node child and may still be waiting for
        // its candidate metadata. Its stop file lets it leave without killing a PID.
        for (const entry of nodeChildren) {
            let outcome = null;
            try { outcome = await closeWithin(entry, 12000); } catch (error) { result.cleanup.nodeChildren.push({ label: entry.label, reaped: false, error: String(error) }); }
            if (outcome && !result.cleanup.nodeChildren.some(row => row.label === entry.label)) {
                result.cleanup.nodeChildren.push({ label: entry.label, pid: entry.child.pid, reaped: true, ...outcome });
            }
        }

        for (const record of held) {
            let identity = record.identity;
            if (!identity && existsSync(record.metadataPath)) {
                try { identity = JSON.parse(readFileSync(record.metadataPath, 'utf8')); record.identity = identity; } catch { }
            }
            if (!identity && record === replacement) {
                const launchInfoPath = path.join(outputDir, 'relaunch-launch-info.json');
                if (existsSync(launchInfoPath)) {
                    try { identity = JSON.parse(readFileSync(launchInfoPath, 'utf8')); record.identity = identity; } catch { }
                }
            }
            let gone = false;
            if (identity) {
                try { gone = await waitForIdentityGone(powerShell, identity, Date.now() + 12000); } catch { gone = false; }
            }
            result.cleanup.held.push({ name: record.name, identity: identity ?? null, stopPath: record.stopPath, stopPresent: existsSync(record.stopPath), exactIdentityGone: gone });
        }

        if (observer) {
            try { result.cleanup.observer = await observer.close(); } catch (error) {
                result.cleanup.observer = observer.descriptor?.cleanup ?? { error: String(error) };
                if (!result.error) result.error = { message: `observer cleanup failed: ${error.message}`, stack: error.stack ?? null };
            }
        }
        result.cleanup.deadlineObserver = deadlineResult ? {
            pid: deadlineResult.pid,
            exitCode: deadlineResult.exitCode,
            reaped: deadlineResult.reaped,
        } : null;
        result.traceAtEnd = traceAtEnd;
        result.finishedAt = new Date().toISOString();
        const cleanupOk = result.cleanup.held.length === held.length && result.cleanup.held.every(item => item.exactIdentityGone) &&
            result.cleanup.nodeChildren.length === nodeChildren.length && result.cleanup.nodeChildren.every(item => item.reaped) &&
            (!observer || result.cleanup.observer?.code === 0 && !result.cleanup.observer?.forced && !result.cleanup.observer?.stopError) &&
            (!deadlineResult || deadlineResult.reaped);
        result.cleanup.passed = cleanupOk;
        result.status = !result.error && cases.length === 4 && cases.every(item => item.status === 'PASS') && cleanupOk ? 'PASS' : 'FAIL';
        if (outputCreated) writeFileSync(path.join(outputDir, 'result.json'), `${JSON.stringify(result, null, 2)}\n`, { flag: 'wx', encoding: 'utf8' });
    }
    return result;
}

async function main () {
    const args = parseArgs(process.argv.slice(2));
    if (args.help) {
        process.stdout.write(`${USAGE}\n`);
        return 0;
    }
    if (args.error) {
        process.stderr.write(`${args.error}\n\n${USAGE}\n`);
        return 2;
    }
    if (!path.isAbsolute(args.output) || !path.isAbsolute(args.powerShell)) {
        process.stderr.write(`Both output and PowerShell paths must be absolute.\n\n${USAGE}\n`);
        return 2;
    }
    if (process.platform !== 'win32') {
        process.stdout.write(`${JSON.stringify({ status: 'SKIP', nativePass: false, platform: process.platform, reason: 'Windows native process-start events and CIM are required; no native fixture ran.' }, null, 2)}\n`);
        return 0;
    }
    if (!existsSync(args.powerShell)) {
        process.stderr.write(`PowerShell executable does not exist: ${args.powerShell}\n`);
        return 2;
    }
    const result = await runFixture({ outputDir: args.output, powerShell: args.powerShell });
    process.stdout.write(`${JSON.stringify({ status: result.status, resultFile: existsSync(path.join(args.output, 'result.json')) ? path.join(args.output, 'result.json') : null, cases: result.cases.map(({ name, status }) => ({ name, status })), cleanup: result.cleanup.passed, error: result.error }, null, 2)}\n`);
    return result.status === 'PASS' ? 0 : 1;
}

main().then(code => { process.exitCode = code; }).catch(error => {
    process.stderr.write(`${error?.stack ?? error}\n`);
    process.exitCode = 1;
});
