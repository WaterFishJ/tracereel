# -*- coding: utf-8 -*-
"""`.qrt` object-graph reader — final layout.

Type-table entry (validated field-by-field against the .NET reference streams
produced by `demo/gen_qrtlike.ps1` / `gen_qrtlike2.ps1`):

    count(4B) | nameLen(1B) | className | memberCount(4B)
              | (nameLen(1B)+memberName)*N
              | binaryTypeEnum(1B)*N | additionalInfos | libraryId(4B)

Object ids are handed out in definition order (no explicit id field).

Value area records:
    0x06 BinaryObjectString | oid(4B) | str
    0x09 MemberReference    | oid(4B)
    0x0A ObjectNull         (1 byte only -- QuickRoute writes the short form)
    0x07 BinaryArray        | oid(4B) | kind(1B) | rank(4B) | lens | [bounds]
                            | elementType(1B) | elementInfo | payload
    0x0D/0x0E null-multiple with 1B/4B count
    0x0B MessageEnd
"""
import struct
import sys

sys.stdout.reconfigure(encoding='utf-8', errors='replace')

PT = {1: ('b', 1), 2: ('B', 1), 3: ('?', 1), 4: ('h', 2), 5: ('i', 4),
      6: ('q', 8), 7: ('f', 4), 8: ('d', 8), 9: ('h', 2), 10: ('i', 4),
      11: ('q', 8), 12: ('d', 8), 13: ('q', 8), 14: ('h', 2), 15: ('h', 2),
      16: ('d', 8), 17: ('b', 1), 18: ('i', 4)}


class Graph:
    def __init__(self, data):
        self.b = data
        self.p = 17                       # 17-byte NRBF serialization header
        self.classes = []
        self.objs = {}
        self.next_id = 1

    # ------------------------------------------------------------ primitives
    def u8(self):
        v = self.b[self.p]; self.p += 1; return v

    def i32(self):
        v = struct.unpack_from('<i', self.b, self.p)[0]; self.p += 4; return v

    def take(self, n):
        v = self.b[self.p:self.p + n]; self.p += n; return v

    def l7(self):
        r = 0; s = 0
        while True:
            x = self.u8()
            r |= (x & 0x7F) << s; s += 7
            if not x & 0x80:
                return r

    def string(self):
        n = self.l7()
        return self.take(n).decode('utf-8', 'replace')

    # ------------------------------------------------------------ type table
    def read_library(self):
        assert self.u8() == 0x0C
        self.i32()
        name = self.string()
        libid = self.u8()
        return name, libid

    def _valid_members_at(self, p, k):
        """True when k length-prefixed ASCII member names start at p."""
        q = p
        for _ in range(k):
            if q >= len(self.b):
                return False
            ml = self.b[q]
            if not (1 <= ml <= 120):
                return False
            raw = self.b[q + 1:q + 1 + ml]
            if len(raw) != ml:
                return False
            try:
                raw.decode('ascii')
            except UnicodeDecodeError:
                return False
            q += 1 + ml
        return True

    def read_entry(self):
        """Type-table entry. Two observed variants:

        A) tag(1B) | oid(4B) | nameLen(1B) | name | mc(1B or 4B) | ...
        B)           nameLen(1B) | name      | mc(1B or 4B) | ...

        All widths are chosen by validating what follows, so both .NET reference
        streams and QuickRoute `.qrt` parse with the same code.
        """
        p0 = self.p
        # Variants, tried in order; each candidate is validated against the bytes
        # that must follow, so the correct one wins unambiguously:
        #   A) tag(1B) | oid(4B) | nameLen(1B) | name | mc(1B|4B) | ... | libraryId(4B)
        #   B)           nameLen(1B) | name      | mc(1B|4B) | ... | libraryId(4B)
        # Entry shape (confirmed against the .NET reference streams):
        #   tag(1B) | oid(4B, signed) | nameLen(7-bit varint) | name
        #           | memberCount(1B or 4B) | memberNames | typeCodes | ...
        # A tag-less variant (no tag/oid) also appears in the .NET streams, so
        # each candidate is validated against the bytes that must follow.
        for tag_len, oid_len, with_lib in ((1, 4, True), (0, 0, True), (1, 4, False)):
            p = p0 + tag_len + oid_len
            if p + 8 > len(self.b):
                continue
            if tag_len == 1 and self.b[p0] not in (0x01, 0x02, 0x03, 0x04, 0x05):
                continue
            if oid_len == 4:
                oid_field = struct.unpack_from('<i', self.b, p0 + tag_len)[0]
                if not (-100000 <= oid_field <= 100000):
                    continue
            save = self.p
            self.p = p
            try:
                nl = self.l7()
            except Exception:
                self.p = save
                continue
            if not (3 <= nl <= 400) or self.p + nl > len(self.b):
                self.p = save
                continue
            raw = self.b[self.p:self.p + nl]
            if not raw[:1].isalpha():
                self.p = save
                continue
            try:
                name = raw.decode('ascii')
            except UnicodeDecodeError:
                self.p = save
                continue
            after_name = self.p + nl
            self.p = save
            for width in (1, 4):
                if width == 1:
                    k = self.b[after_name]
                    mstart = after_name + 1
                else:
                    k = struct.unpack_from('<i', self.b, after_name)[0]
                    mstart = after_name + 4
                if not (0 <= k <= 80) or not self._valid_members_at(mstart, k):
                    continue
                self.p = mstart
                members = [self.string() for _ in range(k)]
                bts = [self.u8() for _ in range(k)]
                addl = []
                for bt in bts:
                    if bt in (0, 7):
                        addl.append(self.u8())
                    elif bt == 3:
                        addl.append(self.string())
                    elif bt == 4:
                        addl.append((self.string(), self.i32()))
                    elif bt in (5, 6):
                        addl.append((self.i32(), self.i32()))
                    else:
                        addl.append(None)
                lib = self.i32() if with_lib else None
                oid = len(self.classes) + 1
                cls = dict(oid=oid, name=name, members=members, btypes=bts,
                           addl=addl, lib=lib, tag_len=tag_len, mc_width=width,
                           with_lib=with_lib)
                self.classes.append(cls)
                return cls
        self.p = p0
        return None

    # ------------------------------------------------------------ value area
    # ------------------------------------------------------------ instances
    def read_instance(self, cls, depth=0):
        """Read one instance of `cls` (ISerializable member values, in order).

        Member values are driven by the declared types, not by record tags:
        primitives sit inline, strings/objects are records.
        """
        vals = {}
        bts = cls.get('btypes') or []
        for i, m in enumerate(cls['members']):
            bt = bts[i] if i < len(bts) else None
            vals[m] = self.read_member(bt, depth + 1)
        inst = dict(cls=cls['name'].rsplit('.', 1)[-1], vals=vals)
        self.objs.setdefault('instances', []).append(inst)
        return inst

    def read_member(self, bt, depth=0):
        if bt == 0:                        # primitive, inline
            # The declared primitive width is not carried per-member in this
            # dialect, so pick the reading that keeps the stream coherent: an
            # `f64` when it yields a finite value, else fall back to a tagged
            # MemberPrimitiveTyped record.
            if self.p + 8 <= len(self.b):
                v = struct.unpack_from('<d', self.b, self.p)[0]
                if -1e12 < v < 1e12:
                    self.p += 8
                    return v
            pt = self.u8()
            fmt, sz = PT.get(pt, ('d', 8))
            return struct.unpack('<' + fmt, self.take(sz))[0]
        if bt == 7:                        # PrimitiveArray -> ArraySinglePrimitive
            tag = self.u8()
            if tag == 0x0F:
                oid = self.i32()
                n = self.i32()
                pt = self.u8()
                fmt, sz = PT.get(pt, ('B', 1))
                raw = self.take(sz * n)
                vals = list(raw) if sz == 1 else list(
                    struct.unpack('<' + fmt * n, raw))
                self.objs[oid] = vals
                return vals
            self.p -= 1
            return self.record(depth)
        if bt in (5, 6):                   # Object/String array
            return self.record(depth)
        return self.record(depth)

    READY = {6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17}

    def record(self, depth=0):
        if depth > 64:
            raise ValueError('depth @%d' % self.p)
        p0 = self.p
        tag = self.b[p0]
        if tag == 0x00:
            # Inline member value for the class just defined (e.g. an enum's
            # `value__`): read the smallest width that keeps the next class
            # definition intact.
            for width in (4, 1, 2, 8):
                cand = p0 + width
                save = self.p
                self.p = cand
                e = self.read_entry()
                self.p = save
                if e is not None:
                    self.p = cand
                    cls = self.classes.pop()
                    return ('value', cls['name'], struct.unpack_from(
                        '<' + {1: 'b', 2: 'h', 4: 'i', 8: 'q'}[width],
                        self.b, p0)[0])
            self.p = p0 + 1
            return None
        if tag not in self.READY:
            e = self.read_entry()
            if e is not None:
                # A class definition may be followed immediately by its instance
                # values (ISerializable writes them right after the definition).
                inst = self.read_instance(e, depth)
                return ('inst', e['name'].rsplit('.', 1)[-1], inst['vals'])
            self.p = p0
            raise ValueError('unknown tag 0x%02X @%d' % (tag, p0))
        self.p = p0 + 1
        if tag == 9:
            return ('ref', self.i32())
        if tag == 10:
            return None
        if tag == 11:
            return ('end',)
        if tag == 12:
            self.i32(); return ('lib', self.string())
        if tag == 6:
            oid = self.i32()
            s = self.string()
            self.objs[oid] = s
            return s
        if tag == 8:
            pt = self.u8()
            fmt, sz = PT[pt]
            return struct.unpack('<' + fmt, self.take(sz))[0]
        if tag == 13:
            return [None] * self.u8()
        if tag == 14:
            return [None] * self.i32()
        if tag == 7:
            oid = self.i32()
            kind = self.u8()
            rank = self.i32()
            lens = [self.i32() for _ in range(rank)]
            if kind in (3, 4, 5):
                [self.i32() for _ in range(rank)]
            bt = self.u8()
            addl = None
            if bt == 0:
                addl = self.u8()
            elif bt == 3:
                addl = self.string()
            elif bt == 4:
                addl = (self.string(), self.i32())
            total = 1
            for x in lens:
                total *= x
            if bt == 0:
                fmt, sz = PT[addl]
                raw = self.take(sz * total)
                vals = list(struct.unpack('<' + fmt * total, raw)) if sz > 1 else list(raw)
                self.objs[oid] = vals
                return vals
            arr = [self.record(depth + 1) for _ in range(total)]
            self.objs[oid] = arr
            return arr
        if tag == 15:
            oid = self.i32(); n = self.i32(); pt = self.u8()
            fmt, sz = PT[pt]
            raw = self.take(sz * n)
            vals = list(struct.unpack('<' + fmt * n, raw)) if sz > 1 else list(raw)
            self.objs[oid] = vals
            return vals
        if tag in (16, 17):
            oid = self.i32(); n = self.i32()
            arr = [self.record(depth + 1) for _ in range(n)]
            self.objs[oid] = arr
            return arr
        raise ValueError('unhandled tag 0x%02X @%d' % (tag, p0))


def run(path, limit=200, verbose=False):
    data = open(path, 'rb').read()
    g = Graph(data)
    libname, libid = g.read_library()
    g.i32()                                   # typeCount
    while True:
        e = g.read_entry()
        if e is None:
            break
    print("file %s (%d bytes)" % (path.rsplit('\\', 1)[-1], len(data)))
    print("  library %r id=%d ; values @%d"
          % (libname[:34], libid, g.p))
    n = 0
    handles = []
    while n < limit and g.p < len(data):
        p0 = g.p
        try:
            rec = g.record(0)
        except Exception as e:
            print("  [%2d] @%-9d STOP: %s" % (n, p0, e))
            break
        if isinstance(rec, tuple) and rec and rec[0] == 'inst':
            name, vals = rec[1], rec[2]
            brief = {}
            for k, v in vals.items():
                if isinstance(v, list):
                    brief[k] = '<%d>' % len(v)
                else:
                    brief[k] = v
            if 'Handle' in name or 'Split' in name:
                handles.append((p0, name, vals))
                print("  ** [%2d] @%-9d..%-9d %s %s" % (n, p0, g.p, name, brief))
            elif verbose or n < 16:
                print("  [%2d] @%-9d..%-9d %s %s" % (n, p0, g.p, name, str(brief)[:88]))
        elif verbose or n < 16:
            print("  [%2d] @%-9d..%-9d %s" % (n, p0, g.p, repr(rec)[:88]))
        n += 1
        if n >= limit:
            break
    print("  -> %d records, %d handle-like instances" % (n, len(handles)))
    return g, handles


if __name__ == '__main__':
    args = [a for a in sys.argv[1:] if not a.startswith('-')]
    verbose = '-v' in sys.argv
    for p in args:
        run(p, verbose=verbose)
        print()
