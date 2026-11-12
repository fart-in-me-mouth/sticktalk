CREATE TABLE id_highwater(singleton INTEGER PRIMARY KEY CHECK(singleton=1),value INTEGER NOT NULL CHECK(value>=0));
INSERT INTO id_highwater VALUES(1,0);
