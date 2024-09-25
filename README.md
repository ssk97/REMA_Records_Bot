Bot for reporting the match results of games played for REMA.
Discord Secret Key set via `DISCORD_TOKEN` environment variable.

Full set of commands:  
`/begin` Begin setting up a new match matrix  
`/add` Add user(s) for setup  
`/create` Create the match results matrix thread in this channel  
`/cancel` Cancel the current match matrix setup  
`/end` End a match matrix, posting final results in this channel  
`/result` Report a match result with arbitrary users for the current results thread  
`/reprocess` Read this channel's matrix info into storage. Also resets unavailable report commands

After a results matrix thread has been created, `/<shortname>` can be also be used to submit match results.
This command only has the participants as possible players to select, whereas the generic `/result` command can select any user.
