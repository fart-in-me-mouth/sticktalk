# Seed one server with a #general channel for two registered users.
# Usage: python -I seed.py <db path> <alice id> <bob id>
import sqlite3, sys
db, alice, bob = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
c = sqlite3.connect(db, timeout=10)
c.execute("INSERT INTO guilds VALUES(10,?,'Test Server',NULL,0)", (alice,))
c.execute("INSERT INTO guild_members VALUES(10,?,'owner',0),(10,?,'member',0)", (alice, bob))
c.execute("INSERT INTO channels(id,kind,guild_id,name,topic,position,created_at) VALUES(11,'text',10,'general','Plans and chatter',0,0)")
c.execute("INSERT INTO read_states(user_id,channel_id) VALUES(?,11),(?,11)", (alice, bob))
c.commit()
print("seeded")
