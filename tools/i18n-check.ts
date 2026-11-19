import { readdirSync, readFileSync, statSync } from "node:fs"
import { basename, join } from "node:path"

const langs = ["en", "ru", "uk"]

function qmlFiles(dir: string): string[] {
    return readdirSync(dir).flatMap(name => {
        const path = join(dir, name)
        if (statSync(path).isDirectory()) return name === "dev" ? [] : qmlFiles(path)
        return /\.(qml|js)$/.test(name) ? [path] : []
    })
}

const unescape = (s: string) => s.replace(/\\(.)/g, "$1")
const xml = (s: string) => s.replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&")

const wanted = new Map<string, string>()
for (const file of qmlFiles("qml")) {
    const context = basename(file).replace(/\.(qml|js)$/, "")
    const re = /qsTr\(\s*"((?:[^"\\]|\\.)*)"(?:\s*,\s*"((?:[^"\\]|\\.)*)")?/g
    for (const m of readFileSync(file, "utf8").matchAll(re)) {
        const key = `${context}\u0000${unescape(m[1])}\u0000${unescape(m[2] ?? "")}`
        wanted.set(key, file)
    }
}

let missing = 0
for (const lang of langs) {
    const ts = readFileSync(`i18n/snellar_${lang}.ts`, "utf8")
    const have = new Set<string>()
    for (const ctx of ts.matchAll(/<context>\s*<name>([^<]*)<\/name>([\s\S]*?)<\/context>/g)) {
        for (const msg of ctx[2].matchAll(/<message[^>]*>([\s\S]*?)<\/message>/g)) {
            const body = msg[1]
            const source = /<source>([\s\S]*?)<\/source>/.exec(body)
            const comment = /<comment>([\s\S]*?)<\/comment>/.exec(body)
            const translation = /<translation([^>]*)>([\s\S]*?)<\/translation>/.exec(body)
            if (!source || !translation || /type="(unfinished|vanished|obsolete)"/.test(translation[1])) continue
            if (!translation[2].trim()) continue
            have.add(`${ctx[1]}\u0000${xml(source[1])}\u0000${xml(comment ? comment[1] : "")}`)
        }
    }
    for (const [key, file] of wanted) {
        if (have.has(key)) continue
        const [context, source] = key.split("\u0000")
        console.log(`${lang}: ${file}: ${context}: "${source}" has no translation`)
        missing++
    }
}

if (missing) {
    console.log(`${missing} missing translations; add them to i18n/snellar_<lang>.ts (docs/i18n.md)`)
    process.exit(1)
}
console.log(`i18n ok: ${wanted.size} strings in ${langs.join(", ")}`)
