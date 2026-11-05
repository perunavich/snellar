.pragma library

function read(url, done) {
    const xhr = new XMLHttpRequest()
    xhr.open("GET", url)
    xhr.onload = () => done(xhr.responseText)
    xhr.send()
}

function readNow(url) {
    const xhr = new XMLHttpRequest()
    xhr.open("GET", url, false)
    xhr.send()
    return xhr.responseText
}

function sizeNow(url) {
    const xhr = new XMLHttpRequest()
    xhr.open("GET", url, false)
    xhr.responseType = "arraybuffer"
    xhr.send()
    return xhr.response ? xhr.response.byteLength : 0
}
