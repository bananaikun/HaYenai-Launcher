#!/usr/bin/env node
/**
 * Local stand-in for the HaYenai update service.
 *
 * Use it to exercise the launcher's connection indicator and update flow without
 * a real server:
 *
 *   HAYENAI_UPDATE_BASE=http://127.0.0.1:8787/ node scripts/dev/update-service-stub.mjs
 *
 * Options:
 *   --port <n>       Port to listen on (default 8787)
 *   --delay <ms>     Delay every response by this many milliseconds (default 0)
 *   --fail <status>  Answer every request with this HTTP status (for example 503)
 *   --tls-selfsigned Serve HTTPS with a throwaway self-signed certificate, to
 *                    check that strict certificate validation reports the
 *                    failure instead of connecting anyway
 *
 * Endpoints:
 *   GET /health             {"ok":true,"ts":<unix>,"minLauncher":"0.1.0"}
 *   GET /api/announcements  {"ok":true,"data":[...]}
 *   GET /api/versions       {"versions":[...]}  (same shape the release scripts emit)
 *   GET /latest[?channel=]  tauri updater manifest (version / notes / platforms)
 *
 * The `--tls-selfsigned` mode needs `openssl` on PATH; it generates a temporary
 * key pair in the system temp directory and deletes it on exit.
 */
import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { createServer as createHttpServer } from 'node:http'
import { createServer as createHttpsServer } from 'node:https'
import { tmpdir } from 'node:os'
import path from 'node:path'

const args = process.argv.slice(2)

/** Reads `--name value` out of argv, or returns the fallback. */
function option(name, fallback) {
    const index = args.indexOf(`--${name}`)
    if (index === -1) return fallback
    const value = args[index + 1]
    if (value === undefined || value.startsWith('--')) return true
    return value
}

const port = Number(option('port', 8787))
const delayMs = Number(option('delay', 0))
const failStatus = args.includes('--fail') ? Number(option('fail', 503)) : 0
const useTls = args.includes('--tls-selfsigned')

if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    console.error(`Invalid --port: ${option('port')}`)
    process.exit(1)
}
if (!Number.isFinite(delayMs) || delayMs < 0) {
    console.error(`Invalid --delay: ${option('delay')}`)
    process.exit(1)
}

const versions = {
    versions: [
        {
            version: '9.9.9-stub',
            artifacts: [
                {
                    kind: 'installer',
                    variant: 'nsis',
                    platform: 'windows',
                    architecture: 'x86_64',
                    relative_path: 'stub/HaYenai-stub.exe',
                    sha256: null,
                    size: 0,
                },
            ],
        },
    ],
}

const announcements = {
    ok: true,
    data: [
        {
            id: 'stub-announcement',
            title: 'Stub announcement',
            body: 'Served by scripts/dev/update-service-stub.mjs',
            level: 'info',
            publishedAt: Math.floor(Date.now() / 1000),
            dismissible: true,
        },
    ],
}

/**
 * Tauri updater manifest. The channel query is echoed into the notes so a run
 * makes it obvious which channel was requested.
 */
const latest = {
    version: '9.9.9-stub',
    notes: 'Stub manifest served by scripts/dev/update-service-stub.mjs',
    pub_date: new Date().toISOString(),
    platforms: {},
}

/** Routes one request to its canned payload. */
function route(url) {
    if (url === '/health' || url === '/health/') {
        return { ok: true, ts: Math.floor(Date.now() / 1000), minLauncher: '0.1.0' }
    }
    if (url.startsWith('/api/announcements')) return announcements
    if (url.startsWith('/api/versions')) return versions
    if (url.startsWith('/latest')) return latest
    return null
}

function handler(request, response) {
    const startedAt = Date.now()
    const url = request.url ?? '/'
    const payload = failStatus ? null : route(url)

    const reply = () => {
        if (failStatus) {
            response.writeHead(failStatus, { 'content-type': 'application/json' })
            response.end(
                JSON.stringify({ ok: false, error: { code: 'stub', message: 'forced failure' } }),
            )
        } else if (payload === null) {
            response.writeHead(404, { 'content-type': 'application/json' })
            response.end(JSON.stringify({ ok: false, error: { code: 'not-found', message: url } }))
        } else {
            response.writeHead(200, { 'content-type': 'application/json' })
            response.end(JSON.stringify(payload))
        }
        const elapsed = Date.now() - startedAt
        const status = response.statusCode
        console.log(
            `${new Date().toISOString()} ${request.method} ${url} -> ${status} (${elapsed} ms)`,
        )
    }

    if (delayMs > 0) setTimeout(reply, delayMs)
    else reply()
}

/** Creates a throwaway self-signed certificate pair. */
function selfSignedCertificate() {
    const directory = mkdtempSync(path.join(tmpdir(), 'hayenai-stub-tls-'))
    const keyPath = path.join(directory, 'key.pem')
    const certPath = path.join(directory, 'cert.pem')

    execFileSync(
        'openssl',
        [
            'req',
            '-x509',
            '-newkey',
            'rsa:2048',
            '-nodes',
            '-keyout',
            keyPath,
            '-out',
            certPath,
            '-days',
            '1',
            '-subj',
            '/CN=127.0.0.1',
            '-addext',
            'subjectAltName=IP:127.0.0.1,DNS:localhost',
        ],
        { stdio: 'ignore' },
    )

    return {
        key: readFileSync(keyPath),
        cert: readFileSync(certPath),
        cleanup: () => rmSync(directory, { recursive: true, force: true }),
    }
}

let server
let cleanupCertificate = () => {}

if (useTls) {
    let pair
    try {
        pair = selfSignedCertificate()
    } catch (error) {
        console.error('Could not generate a self-signed certificate; is `openssl` on PATH?')
        console.error(String(error))
        process.exit(1)
    }
    cleanupCertificate = pair.cleanup
    server = createHttpsServer({ key: pair.key, cert: pair.cert }, handler)
} else {
    server = createHttpServer(handler)
}

server.listen(port, '127.0.0.1', () => {
    const scheme = useTls ? 'https' : 'http'
    console.log(`update-service stub listening on ${scheme}://127.0.0.1:${port}/`)
    console.log(
        `  delay=${delayMs}ms fail=${failStatus || 'none'} tls=${useTls ? 'self-signed' : 'off'}`,
    )
    console.log(`  try: HAYENAI_UPDATE_BASE=${scheme}://127.0.0.1:${port}/ <launcher>`)
})

function shutdown() {
    server.close(() => {
        cleanupCertificate()
        process.exit(0)
    })
}

process.on('SIGINT', shutdown)
process.on('SIGTERM', shutdown)
