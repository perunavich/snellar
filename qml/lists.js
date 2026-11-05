.pragma library

function fold(s) {
    return String(s || "").normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLocaleLowerCase()
}

function haystack(item) {
    return fold([item.title, item.name, (item.artists || []).map(a => a.name).join(" "), item.album ? item.album.name : "", item.description].join(" "))
}

function value(item, key, index) {
    switch (key) {
    case "title": return fold(item.title || item.name)
    case "artist": return fold(item.artists && item.artists.length ? item.artists[0].name : "")
    case "album": return fold(item.album ? item.album.name : "")
    case "duration": return item.duration || 0
    case "year": return parseInt(item.year) || 0
    case "added": return item.at !== undefined ? -item.at : index
    }
    return index
}

function compare(a, b) {
    if (typeof a === "number") return a - b
    if (!a !== !b) return a ? -1 : 1
    return a.localeCompare(b, undefined, { numeric: true })
}

function arrange(items, query, key, reversed, natural) {
    const words = fold(query).split(/\s+/).filter(w => w.length)
    let list = (items || []).map((x, i) => ({ x: x, i: i }))
    if (words.length) list = list.filter(e => { const hay = haystack(e.x); return words.every(w => hay.indexOf(w) >= 0) })
    if (key !== natural) list.sort((a, b) => compare(value(a.x, key, a.i), value(b.x, key, b.i)) || a.i - b.i)
    if (reversed) list.reverse()
    return list.map(e => e.x)
}
