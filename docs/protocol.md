# Xfire Protocol Overview

This is a contributor-friendly overview of the Xfire binary protocol. For the complete byte-level
specification, see `.claude/docs/xfire-protocol.md`.

## Basics

Xfire uses a **custom binary protocol** over TCP port **25999**. It is not based on XMPP, IRC, or
any standard IM protocol — it's entirely proprietary, reverse-engineered by the open-source
community between 2005–2010.

- **Server**: `cs.xfire.com:25999`
- **Byte order**: Little-endian
- **Strings**: UTF-8
- **Handshake**: Client sends the 4 ASCII bytes `"UA01"`

## Packet Structure

Every message follows this format:

```
┌──────────┬──────────┬────────────┬─────────────────┐
│ size     │ type_id  │ attr_count │ attributes...   │
│ 2 bytes  │ 2 bytes  │ 1 byte     │ variable        │
│ uint16   │ uint16   │ uint8      │                 │
└──────────┴──────────┴────────────┴─────────────────┘
```

- `size`: Total message size including header
- `type_id`: Identifies the packet type (e.g., 1 = LoginRequest, 130 = LoginSuccess)
- `attr_count`: Number of attribute key-value pairs that follow
- Client-to-server packet IDs are 1–127
- Server-to-client packet IDs are 128+

## Attributes

Each attribute is a typed key-value pair:

```
[1 byte: name_length][N bytes: name][1 byte: type_id][value_data]
```

| Type | ID   | Encoding |
|------|------|----------|
| String | 0x01 | uint16 length + UTF-8 bytes |
| Int32 | 0x02 | 4 bytes LE |
| Session ID | 0x03 | 16 raw bytes |
| List | 0x04 | 1 byte item_type + uint16 count + items |
| Map (string keys) | 0x05 | 1 byte count + entries |
| DID | 0x06 | 16 raw bytes |
| Map (int keys) | 0x09 | 1 byte count + entries |

## Authentication

```
1. Client connects, sends "UA01"
2. Client sends ClientInfo (packet 18) and ClientVersion (packet 3)
3. Server sends LoginChallenge (packet 128) with a 40-byte salt
4. Client sends LoginRequest (packet 1) with username + hashed password
5. Server responds with LoginSuccess (packet 130) or LoginFailure (packet 129)
```

### Password Hashing

```
step1 = SHA1(username + password + "UltimateArena")
step2 = SHA1(step1 + server_salt)
```

The constant `"UltimateArena"` is the original company name (Ultimate Arena, Inc., founded 2002).
`step1` is stored locally in the client config. `step2` is what gets sent over the wire.

## Key Packet Types

### Client -> Server
| ID | Name | Purpose |
|----|------|---------|
| 1 | LoginRequest | Authenticate with username + hashed password |
| 2 | ChatMessage | Send an IM (via `peermsg` map) |
| 3 | ClientVersion | Report client version |
| 4 | GameStatus | Report current game + server |
| 6 | AddFriend | Send friend request |
| 9 | RemoveFriend | Remove a friend |
| 11 | SetStatus | Change online status + message |
| 12 | KeepAlive | Connection heartbeat |

### Server -> Client
| ID | Name | Purpose |
|----|------|---------|
| 128 | LoginChallenge | Provides 40-byte salt for auth |
| 130 | LoginSuccess | Returns userid, session ID, nickname |
| 131 | FriendList | Full friends list (userids, names, nicks) |
| 132 | SessionAssign | Maps online friends to session IDs |
| 133 | ChatMessage | Incoming IM (via `peermsg` map) |
| 135 | FriendGameInfo | Friends' game status (game ID, server IP) |
| 138 | FriendRequest | Incoming friend request |

## Chat Messages

Chat uses a `peermsg` map attribute with subtypes:

| msgtype | Purpose |
|---------|---------|
| 0 | Content message (actual text) |
| 1 | Acknowledgement |
| 2 | Client info exchange (for P2P setup) |
| 3 | Typing notification |

## P2P Communication

Direct messaging can bypass the server via UDP:
1. Both clients exchange connection info (IP, port) via server-relayed peermsg type 2
2. UDP hole punching establishes a direct channel
3. Messages flow directly between clients
4. Falls back to server relay if P2P fails

## Further Reading

- Full byte-level spec: `.claude/docs/xfire-protocol.md`
- OpenFire protocol docs: https://github.com/iainmcgin/openfire
- gfire source (most complete implementation): https://github.com/gfireproject/gfire
- PFire server emulator: https://github.com/darcymiranda/PFire
