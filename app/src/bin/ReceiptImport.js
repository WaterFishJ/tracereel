.pragma library

function parseTime(value) {
    var text = String(value).trim().replace(/：/g, ":").replace(/,/g, ".")
    if (!/^\d+(?::\d{1,2}){0,2}(?:\.\d{1,3})?$/.test(text)) return NaN
    var parts = text.split(":")
    var result = 0
    for (var i = 0; i < parts.length; i++) {
        var n = Number(parts[i])
        if (i > 0 && n >= 60) return NaN
        result = result * 60 + n
    }
    return Math.round(result * 1000) / 1000
}

function formatTime(seconds, template) {
    var ms = Math.round(seconds * 1000)
    if (String(template).replace(/：/g, ":").indexOf(":") < 0) return String(ms / 1000)
    var hours = Math.floor(ms / 3600000), minutes = Math.floor(ms / 60000) % 60
    var secs = Math.floor(ms / 1000) % 60
    var fraction = ms % 1000 ? "." + ("00" + ms % 1000).slice(-3).replace(/0+$/, "") : ""
    var tail = ("0" + secs).slice(-2) + fraction
    // Keep mm:ss receipts in that notation even when minutes exceed 59.
    if (String(template).split(":").length === 2) return Math.floor(ms / 60000) + ":" + tail
    return hours ? hours + ":" + ("0" + minutes).slice(-2) + ":" + tail : minutes + ":" + tail
}

function normalizeTime(text) {
    text = String(text).trim().replace(/：/g, ":").replace(/,/g, ".")
    if (text.indexOf(":") < 0) return formatTime(parseTime(text), text)
    var parts = text.split(":")
    while (parts.length > 2 && Number(parts[0]) === 0) parts.shift()
    parts[0] = String(Number(parts[0]))
    return parts.join(":")
}

function build(rows, timeColumn, relative, ordinalColumn, codeColumn) {
    if (timeColumn < 0) return {error: "请选择时间列", times: []}
    var total = 0, previous = 0, times = [], display = [], codes = []
    for (var i = 0; i < rows.length; i++) {
        var row = rows[i], value = parseTime(row.cells[timeColumn] || "")
        if (!isFinite(value)) {
            if (row.include) return {error: "第 " + (i + 1) + " 行时间无效，请修改或取消导入该行", times: []}
            continue
        }
        if (relative) total = Math.round((total + value) * 1000) / 1000
        else total = value
        // Unchecked punches still contribute to elapsed time in relative mode.
        // This preserves subsequent boundaries when a punch is unchecked.
        if (!row.include || total === 0) continue
        if (total <= previous) return {error: "第 " + (i + 1) + " 行累计时间未递增，请核对时间类型或合并重复打卡", times: []}
        var importedTime = relative && Math.abs(total - previous - value) > .0001
                ? formatTime(total - previous, row.cells[timeColumn]) : normalizeTime(row.cells[timeColumn])
        previous = total
        times.push(importedTime)
        codes.push(codeColumn >= 0 ? String(row.cells[codeColumn] || "").trim() : "")
        var label = "第 " + times.length + " 段"
        if (ordinalColumn >= 0 && row.cells[ordinalColumn]) label += "（原分段号 " + row.cells[ordinalColumn] + "）"
        if (codeColumn >= 0 && row.cells[codeColumn]) label += " 点号 " + row.cells[codeColumn]
        display.push(label + "：" + importedTime)
    }
    return {error: times.length ? "" : "没有有效分段", times: times, codes: codes, relative: relative, finish: formatTime(previous,"0:00:00"), display: display}
}

function neighbor(rows, index, direction) {
    if (!rows[index] || (direction !== -1 && direction !== 1)) return -1
    var next = index + direction
    return next >= 0 && next < rows.length ? next : -1
}

function suspects(rows, codeColumn) {
    return rows.map(function(row, index) {
        if (codeColumn < 0 || !row.include) return false
        var code = String(row.cells[codeColumn] || "").trim()
        if (!code) return false
        return [-1,1].some(function(direction) {
            var next = neighbor(rows,index,direction)
            return next >= 0 && rows[next].include && String(rows[next].cells[codeColumn] || "").trim() === code
        })
    })
}

function mergeRow(rows, index, direction, timeColumn, relative) {
    var target = neighbor(rows,index,direction)
    if (target < 0) return "没有可合并的相邻行"
    if (timeColumn < 0) return "请先选择时间列和时间类型"
    var sourceTime = parseTime(rows[index].cells[timeColumn] || "")
    var targetTime = parseTime(rows[target].cells[timeColumn] || "")
    if (!isFinite(sourceTime) || !isFinite(targetTime)) return "两行的时间必须有效，请先核对时间列"
    // Cumulative times are endpoints, so keep the later row's endpoint.
    // Relative durations add in either direction; never sum absolute endpoints.
    var value = relative ? sourceTime + targetTime : (direction < 0 ? sourceTime : targetTime)
    // Replace the record too, so cell bindings refresh even when the target
    // stays at the same row index after an upward merge.
    rows[target] = JSON.parse(JSON.stringify(rows[target]))
    rows[target].cells[timeColumn] = formatTime(value, rows[target].cells[timeColumn])
    rows.splice(index,1)
    return ""
}
