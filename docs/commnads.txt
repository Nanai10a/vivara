- join (ch)                                      effect:songbird,connector
- leave                                          effect:songbird,connector
- enqueue:url (url)                              effect:connector
  - integrated with "play"

- show:current                                   read:connector
- show:queue [page(1)]                           read:connector
- show:history [page(1)]                         read:connector

- pause                                          effect:connector
- resume                                         effect:connector
- slide (from) (to)                              effect:songbird,connector
- drop [items(1) or range]                       effect:songbird,connector
  - clear queue: "skip --range .."
- loop [index(0) or range]  // plan some patch?  effect:connector
- shuffle                                        effect:connector
- volume (value)                                 effect:connector
- volume_current (value)                         effect:connector
- fix                       // no planned        effect:songbird,connector
- seek (absolute or relative)
- stop                                           effect:songbird,connector

effect:connector           ControlAction
effect:songbird,connector  CallAction
read:connector             GetStatus

- time specifier
  - schema
    - "[+-]?(%dy)?(%dM)?(%dd)?..."
  - units
    - y(ear)          >>  y
    - M(onth)         >>  M
    - d(ay)           >>  d
    - h(our)          >>  h
    - m(inute)        >>  m
    - s(econd)        >>  s
    - m(illi)s(econd) >> ms
    - n(ano)s(econd)  >> ns
