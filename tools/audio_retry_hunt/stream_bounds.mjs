// Private ROM input -> structural counts only. Never emits resource bytes or bytecode.
// VM semantics: G/src/asm/sound.s:833-1053. Resource spans: public tje-mem catalogue.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
const [romPath, revision, output] = process.argv.slice(2);
assert(['rev02', 'rev00'].includes(revision), 'explicit pinned revision required');
const image = readFileSync(romPath);
const delta = revision === 'rev00' ? -0x70 : 0;
const song = 0x3d696 + delta;
const sfx = 0x3cd20 + delta;
const starts = [...Array.from({ length: 90 }, (_, i) => song + i * 38), ...Array.from({ length: 28 }, (_, i) => sfx + i * 38)];
const roots = [...new Set(starts.flatMap(p => [image.readUInt32BE(p), image.readUInt32BE(p + 4)]).filter(Boolean))];
const regions = [[0x3c4a3, 0x3cd20], [0x3d148, 0x3d666], [0x3f7fe, 0x44d8a]].map(pair => pair.map(p => p + delta));
const nodes = new Map();
const todo = [...roots];
while (todo.length) {
    const pc = todo.pop();
    if (nodes.has(pc)) continue;
    assert(regions.some(([a, b]) => a <= pc && pc < b), 'stream edge outside catalogued music resources');
    const op = image[pc];
    let next = pc + 1;
    let edges = [];
    let callback = null;
    if (op >= 128) edges = [next];
    else if (op === 0) { /* Stop stream. */ }
    else if (op === 1 || op === 3 || op === 5) { next += 2; edges = [next]; }
    else if (op === 4) { next++; edges = [next]; }
    else if (op === 6) {
        const displacement = image.readInt16BE(next);
        next += 2;
        // Both counter outcomes are retained, even if one is unreachable.
        edges = [next, next + 3 + displacement];
    } else if (op === 7) {
        if (next % 2) next++;
        callback = image.readUInt32BE(next);
        next += 4;
        edges = [next];
    } else if (op === 8) {
        const displacement = image.readInt16BE(next);
        next += 2;
        edges = [next + displacement];
    } else {
        assert(op <= 12, 'reserved VM opcode reached');
        edges = [next];
    }
    nodes.set(pc, { op, edges, callback });
    todo.push(...edges);
}
function maxBurst(weight) {
    const memo = new Map();
    const active = new Set();
    function visit(pc) {
        if (memo.has(pc)) return memo.get(pc);
        assert(!active.has(pc), 'unwaited bytecode cycle');
        const node = nodes.get(pc);
        if (node.op >= 128) return 0;
        active.add(pc);
        const value = weight(node.op) + Math.max(0, ...node.edges.map(visit));
        active.delete(pc);
        memo.set(pc, value);
        return value;
    }
    return Math.max(...[...nodes.keys()].map(visit));
}
function reachable(root) {
    const seen = new Set();
    const pending = [root];
    while (pending.length) {
        const pc = pending.pop();
        if (!pc || seen.has(pc)) continue;
        seen.add(pc);
        pending.push(...nodes.get(pc).edges);
    }
    return seen;
}
const pcmRecords = [];
const callbackSlots = Array.from({ length: 15 }, () => []);
for (const [index, start] of starts.entries()) {
    const stream = reachable(image.readUInt32BE(start + 4));
    if ([...stream].some(pc => nodes.get(pc).op >= 9 && nodes.get(pc).op <= 12)) {
        pcmRecords.push({ bank: index < 90 ? Math.floor(index / 6) : 'sfx', slot: index < 90 ? index % 6 : index - 90, channel: image[start + 12] });
    }
    if (index < 90 && (image.readUInt32BE(start + 8) !== 0 || [...stream].some(pc => nodes.get(pc).callback))) {
        callbackSlots[Math.floor(index / 6)].push(index % 6);
    }
}
function branchProof(entry, owner, branch, delay) {
    const first = image.readUInt16BE(entry);
    assert(first === 0x33fc && image.readUInt16BE(entry + 2) === 0 && image.readUInt32BE(entry + 4) === 0xa11100, 'retry does not start with source BUSREQ release');
    const moveq = image.readUInt16BE(entry + 8);
    assert((moveq & 0xff00) === 0x7000 && (moveq & 0xff) === delay, 'retry counter differs from source');
    const dbra = image.readUInt16BE(entry + 10);
    assert(dbra === 0x51c8 && entry + 12 + image.readInt16BE(entry + 12) === entry + 10, 'retry DBRA does not loop on D0');
    assert(entry + 14 === owner, 'retry does not fall through directly into owner');
    const bne = image.readUInt16BE(branch);
    assert((bne & 0xff00) === 0x6600 && (bne & 0xff) !== 0, 'owner lock branch is not BNE.s');
    const displacement = (bne & 0x80) ? (bne & 0xff) - 256 : bne & 0xff;
    assert(branch + 2 + displacement === entry, 'owner BNE does not target retry');
    return { retry_entry: entry, owner_entry: owner, owner_busy_branch: branch, branch: 'BNE.s', branch_target: entry, fragment_bytes: 14, dbra_iterations: delay + 1, entry_kind: 'executed instruction target, not JSR' };
}
const report = {
    revision,
    method: 'Read-only conservative control-flow graph; both VM6 counter outcomes; wait tokens end a per-tick burst but their continuations stay reachable. Every source span is private ROM input; only derived structural counts are output.',
    template_records: starts.length,
    stream_roots: roots.length,
    decoded_instructions: nodes.size,
    invalid_opcodes_or_edges: 0,
    unwaited_cycles: 0,
    max_commands_between_waits: maxBurst(op => op === 0 ? 0 : 1),
    max_note_commands_between_waits: maxBurst(op => op === 1 ? 1 : 0),
    max_pcm_commands_between_waits: maxBurst(op => op >= 9 && op <= 12 ? 1 : 0),
    pcm_records: pcmRecords,
    callback_slots: callbackSlots,
    fragments: [branchProof(0x3f61c + delta, 0x3f62a + delta, 0x3f64e + delta, 23), branchProof(0x3f79c + delta, 0x3f7aa + delta, 0x3f7ce + delta, 123)],
    limitations: 'This is a structural bytecode bound, not a complete cycle bound for every graphics/audio callback and not an all-input unreachability proof.'
};
writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify({ revision, template_records: starts.length, decoded_instructions: nodes.size, max_commands_between_waits: report.max_commands_between_waits, pcm_records: pcmRecords.length, fragments_verified: report.fragments.length }));
