#!/usr/bin/env python3
"""Random emacs-editing cases (undo, kill ring, overwrite, numeric args,
keyboard macros, typeahead) ending in <dumpfull>.

    python3 impl/tools/fuzzgen/emacs.py SEED N OUT.case
    ./rlt OUT.case --record --repeat 2 -j 4     # expectations from the reference
    ./rlt --impl rust OUT.case -j 4             # compare

Keep OUT outside tests/ (e.g. /tmp); turn real mismatches into named cases.
"""
import random,sys
seed=int(sys.argv[1]); n=int(sys.argv[2]); outpath=sys.argv[3]
R=random.Random(seed)
TEXT=list("abcdefxyz  .,-_/()'\"0123")+["é","日","ß"]
KEYS=[("\\C-a",3),("\\C-e",3),("\\C-b",5),("\\C-f",5),("\\M-b",3),("\\M-f",3),("\\C-d",3),("<bs>",4),
("\\C-k",3),("\\C-u",2),("\\C-w",3),("\\M-d",3),("\\M-\\C-?",2),("\\C-y",4),("\\M-y",3),("\\C-t",3),("\\M-t",3),
("\\M-u",2),("\\M-l",2),("\\M-c",2),("\\C-_",4),("\\M-r",1),("\\C-x\\C-x",2),("\\M-1",1),("\\M-2",2),
("\\M-3",1),("\\M--",2),("\\M-0",1),("\\C-]a",1),("\\M-\\C-]b",1),("\\C-x(",1),("\\C-x)",1),("\\C-xe",1),("\\M-\\\\",1),
("\\C-q\\C-a",1),("<ins>",1),("\\C-x\\C-u",1),("\\M-\\t",1),("\\C-v\\t",1),("\\M-.",0),("\\C-g",1),("\\C-xc",1),("\\C-xw",1),("\\C-xf",1),("\\C-xr",1),("\\C-xk",1),("\\M-x",0)]
keys=[k for k,w in KEYS for _ in range(w)]
out=[]
out.append('tags: efuzz\n--- inputrc\n"\\C-xc": copy-region-as-kill\n"\\C-xw": copy-backward-word\n"\\C-xf": copy-forward-word\n"\\C-xr": kill-region\n"\\C-xk": kill-whole-line\n--- script\nreadline "> "\n')
for i in range(n):
    seq=[]
    for _ in range(R.randint(3,25)):
        r=R.random()
        if r<0.35: seq.append(''.join(R.choice(TEXT) for _ in range(R.randint(1,5))).replace(' ','\\s'))
        else: seq.append(R.choice(keys))
        if R.random()<0.5: seq.append('<pause>')
    out.append(f"=== e{seed}-{i}\nkeys: {''.join(seq)}<dumpfull>\\r\n")
open(outpath,'w').write('\n'.join(out))
