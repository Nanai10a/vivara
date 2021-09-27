- join (ch)                                   effect:songbird,connector
- leave                     // uncomplete     effect:songbird,connector
- queue:url (url)                             effect:connector

- show:current                                read:connector
- show:queue [page(1)]                        read:connector
- show:history [page(1)]                      read:connector

- play [url]                // added option   effect:songbird,connector
- pause                                       effect:connector
- resume                                      effect:connector
- skip [items(1) or range]                    effect:songbird,connector
- loop [index(0) or range]                    effect:connector
- shuffle                                     effect:connector
- volume (value)                              effect:connector
- volume_current (value)                      effect:connector
- stop                                        effect:songbird,connector

effect:connector           ControlAction
effect:songbird,connector  CallAction
read:connector             GetStatus
