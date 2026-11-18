CREATE TABLE runtime_installations (id TEXT PRIMARY KEY, runtime TEXT NOT NULL, version TEXT NOT NULL, record TEXT NOT NULL, UNIQUE(runtime,version,id));
