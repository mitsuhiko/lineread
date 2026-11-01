#!/usr/bin/env python3
"""Random emacs editing + history navigation cases (history preload,
revert-all-at-newline, history-preserve-point, readline or callback mode).

    python3 impl/tools/fuzzgen/emacs_history.py SEED N OUT.case
    ./rlt OUT.case --record --repeat 2 -j 4     # expectations from the reference
    ./rlt --impl rust OUT.case -j 4             # compare

Keep OUT outside tests/ (e.g. /tmp); turn real mismatches into named cases.
"""
import random,sys
seed=int(sys.argv[1]); n=int(sys.argv[2]); outpath=sys.argv[3]
R=random.Random(seed)
TEXT=list("abcxyz  .-_/0123")+["é","日"]
KEYS=[("\\C-a",3),("\\C-e",3),("\\C-b",4),("\\C-f",4),("\\M-b",2),("\\M-f",2),("\\C-d",2),("<bs>",4),
("\\C-k",3),("\\C-u",2),("\\C-w",2),("\\M-d",2),("\\C-y",3),("\\M-y",2),("\\C-t",2),("\\M-t",1),
("\\M-u",1),("\\C-_",5),("\\M-r",3),("\\C-x\\C-u",2),("\\M-2",1),("\\M--",1),
("\\C-p",6),("\\C-n",5),("<up>",3),("<down>",3),("\\M-<",2),("\\M->",2),("\\r",3),("<ins>",1),("\\C-x(",1),("\\C-x)",1),("\\C-xe",1)]
keys=[k for k,w in KEYS for _ in range(w)]
out=[]
for i in range(n):
    rc=[]
    if R.random()<0.3: rc.append("set revert-all-at-newline on")
    if R.random()<0.3: rc.append("set history-preserve-point on")
    hist=R.sample(["first line","second","third one","échelle","x y z","make all"],R.randint(1,4))
    mode=R.choice(["readline","readline","callback"])
    seq=[]
    for _ in range(R.randint(3,25)):
        r=R.random()
        if r<0.3: seq.append(''.join(R.choice(TEXT) for _ in range(R.randint(1,4))).replace(' ','\\s'))
        else: seq.append(R.choice(keys))
        if R.random()<0.5: seq.append('<pause>')
    body=f"=== h{seed}-{i}\n--- inputrc\n"+"\n".join(rc)+"\n--- script\n"+"".join(f'add_history "{h}"\n' for h in hist)+f'{mode} "> " 4 addhist\nhistory_dump\n--- keys\n'+''.join(seq)+"<dumpfull>\\r\n"
    out.append(body)
open(outpath,'w').write('tags: hfuzz\n\n'+'\n'.join(out))
