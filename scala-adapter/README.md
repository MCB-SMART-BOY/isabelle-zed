# Isabelle Scala Adapter

NDJSON adapter that communicates with the Isabelle/PIDE system.

## Build

```bash
sbt compile
```

## Run

### Mock Mode (for CI/testing)

```bash
sbt "run --mock"
```

### Stdin/Stdout Mode

```bash
sbt run
```

### Socket Mode

```bash
sbt "run --socket=localhost:9876"
```

## Protocol

### Input Messages

**document.push**
```json
{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}
```

**document.check**
```json
{"id":"msg-0002","type":"document.check","session":"s1","version":1,"payload":{"uri":"file:///test.thy","version":1}}
```

**markup**
```json
{"id":"msg-0003","type":"markup","session":"s1","version":1,"payload":{"uri":"file:///test.thy","offset":{"line":5,"col":10},"info":""}}
```

### Output Messages

**diagnostics**
```json
{"id":"msg-0004","type":"diagnostics","session":"s1","version":1,"payload":{"diagnostics":[{"uri":"file:///test.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}}
```

## Test

```bash
sbt test
```

## Architecture

This adapter is designed to work with the PIDE framework (Prover IDE).

References:
- Wenzel, Makarius. "Isabelle/jEdit — a Prover IDE within the PIDE framework." (2012)
- Isabelle System Manual: https://isabelle.in.tum.de/doc/system.pdf

## Dependencies

- Scala 3.4.0
- sbt 1.9.9
- json4s-native 4.0.6
- json4s-jackson 4.0.6
