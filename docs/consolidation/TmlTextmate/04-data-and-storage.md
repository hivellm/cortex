# TmlTextmate — Data & Storage

## Grammar File

**Path**: `syntaxes/tml.tmLanguage.json`

**Format**: TextMate Language (JSON)

**Size**: ~8 KB (compressed pattern definitions)

**Schema**: https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json

## File Structure

```
syntaxes/
└── tml.tmLanguage.json          Main grammar definition
```

## Lexicon Coverage

### Keywords (49 total)

**Declaration (11)**: func, type, enum, union, behavior, impl, mod, pub, use, crate, decorator, namespace, class, interface

**Control (15)**: if, then, else, when, loop, while, for, in, to, through, break, continue, return, do, throw, move, async, await, yield

**Operators (10)**: and, or, not, xor, shl, shr, as, is, ref, mut

**Other (13)**: let, var, const, this, Self, with, where, dyn, lowlevel, quote, override, virtual, abstract, sealed, extends, implements, protected, private, static, new, prop, life, volatile

### Primitive Types (12)

I8, I16, I32, I64, I128, U8, U16, U32, U64, U128, F32, F64, Bool, Str, Char, Unit, Never, RawPtr

### Built-in Types (42)

Maybe, Outcome, Result, Option, List, Vec, ArrayList, HashMap, HashSet, HashSetIter, HashMapIter, Buffer, Heap, Shared, Sync, Arc, Mutex, RwLock, BTreeMap, BTreeSet, BinaryHeap, Deque, Queue, Stack, LinkedList, Iterator, IntoIterator, FromIterator, Text, String, Regex, Uuid, Duration, Instant, DateTime, SystemTime, Layout, Allocator, Ordering, Stream, Future, Pin

### Constants (variants)

- Boolean: true, false
- Null: null, Nothing
- Maybe: Just, Ok, Err, Some, None
- Ordering: Less, Equal, Greater

## Storage Considerations

- **Version control**: tracked in git (text-based, diffs readable)
- **Immutability**: changes require manual edits (no code generation)
- **Validation**: by TextMate schema validator
- **Caching**: VSCode caches compiled grammar after first load; invalidated on `.json` change
