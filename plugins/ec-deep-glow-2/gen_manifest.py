#!/usr/bin/env python3
"""manifest.json -> manifest.h  (run before build.sh)"""
import json
m = json.load(open('manifest.json'))
s = json.dumps(m, separators=(', ', ': '))
esc = lambda t: t.replace('\\', '\\\\').replace('"', '\\"')
with open('manifest.h', 'w') as f:
    f.write('/* generated from manifest.json by gen_manifest.py */\nstatic const char MANIFEST[] =\n')
    for i in range(0, len(s), 80):
        f.write('  "%s"\n' % esc(s[i:i + 80]))
    f.write(';\n#define MANIFEST_LEN ((int)(sizeof(MANIFEST)-1))\n')
print(len(s), 'bytes,', len(m['params']), 'param entries')
