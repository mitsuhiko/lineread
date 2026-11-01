# Reference coverage (phase 3 final)

Line/branch/function coverage of the GNU Readline 8.3 reference by the whole
suite (`./rlt --coverage`, macOS arm64, before the clean-room cut). Kept for
the record; coverage cannot be measured after the cut.

```
file                              lines         branches    functions
bind.c                   96.0% 1820/1896   85.8% 1103/1285  100.0%
callback.c               94.7%  198/209    70.4%  112/159   100.0%
colors.c                 92.7%  178/192    85.1%   97/114   100.0%
complete.c               95.1% 1634/1718   79.6%  950/1194  100.0%
display.c                90.3% 1880/2083   73.3% 1164/1587   93.9%
funmap.c                100.0%   38/38     91.7%   11/12    100.0%
histexpand.c             98.5% 1080/1097   85.3%  719/843   100.0%
histfile.c               82.2%  364/443    71.4%  205/287    90.9%
history.c                91.0%  325/357    72.0%  134/186    93.9%
histsearch.c             98.5%  194/197    91.6%   98/107   100.0%
input.c                  84.3%  398/472    64.4%  154/239    95.8%
isearch.c                94.3%  562/596    76.1%  309/406   100.0%
keymaps.c                98.6%   69/70     92.3%   24/26    100.0%
kill.c                   98.7%  520/527    77.1%  215/279    96.4%
macro.c                  91.0%  141/155    65.2%   43/66     92.3%
mbutil.c                 79.8%  332/416    63.2%  213/337    88.2%
misc.c                   96.6%  449/465    89.2%  166/186   100.0%
nls.c                    55.9%   62/111    64.6%   42/65     85.7%
parens.c                 97.7%   84/86     94.9%   37/39    100.0%
parse-colors.c           94.8%  292/308    80.5%  169/210   100.0%
posixtime.h             100.0%    5/5         -     0/0     100.0%
readline.c               94.2%  668/709    78.0%  287/368    97.0%
rlmbutil.h              100.0%   28/28    100.0%  186/186   100.0%
rltty.c                  89.3%  275/308    65.8%   77/117   100.0%
search.c                 95.6%  395/413    81.6%  186/228   100.0%
shell.c                  44.8%   30/67     29.4%    5/17     60.0%
signals.c                89.3%  316/354    78.7%  107/136    90.0%
terminal.c               82.0%  327/399    60.2%  153/254    77.4%
text.c                   90.7% 1295/1427   80.2%  599/747    97.3%
tilde.c                  98.1%  159/162    84.2%   64/76    100.0%
undo.c                   76.6%  157/205    72.6%   61/84     76.9%
util.c                   78.2%  147/188    56.7%   68/120    78.9%
vi_mode.c                95.5% 1491/1561   77.6%  827/1066  100.0%
xfree.c                 100.0%    4/4     100.0%    2/2     100.0%
xmalloc.c                70.6%   12/17     66.7%    4/6      66.7%
TOTAL                    92.2% 15929/17283  77.9% 8591/11034  95.5%
```

Most of the remaining uncovered lines are unreachable through the public API
on these platforms: dead helper functions, code for platforms without
`select`/`pselect`, terminal-capability branches that no terminfo entry in
use triggers, I/O error paths (failing `write`/`mmap`/`ftruncate`), message
catalog support (nls.c) and shell helpers (shell.c) that only bash uses.
