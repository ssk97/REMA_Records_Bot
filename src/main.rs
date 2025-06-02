#![allow(clippy::get_first, clippy::get_last_with_len)] //Clearer when also getting the 2nd or 2nd to last
use std::env;

use serenity::all::*;
use anyhow::{Result, Context as _, anyhow}; //overrides serenity Result

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::LazyLock;
use dashmap::DashMap;
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
enum MatchResult{
    NotPlayed, TwoZero, TwoOne, OneTwo, ZeroTwo, Unplayable
}
impl MatchResult{
    fn get(result: &str) -> Self{
        match result{
            "2-0"|":full_moon:" => Self::TwoZero,
            "2-1"|":waning_gibbous_moon:" => Self::TwoOne,
            "1-2"|":waxing_crescent_moon:" => Self::OneTwo,
            "0-2"|":new_moon:" => Self::ZeroTwo,
            "0-0"|":cloud:" => Self::NotPlayed,
            _ => Self::Unplayable,
        }
    }
    fn render(&self) -> &str{
        match self{
            Self::NotPlayed => ":cloud:",
            Self::TwoZero => ":full_moon:",
            Self::TwoOne => ":waning_gibbous_moon:",
            Self::OneTwo => ":waxing_crescent_moon:",
            Self::ZeroTwo => ":new_moon:",
            Self::Unplayable => ":black_small_square:"
        }
    }
    fn invert(&self) -> Self{
        match self{
            Self::NotPlayed => Self::NotPlayed,
            Self::TwoZero => Self::ZeroTwo,
            Self::TwoOne => Self::OneTwo,
            Self::OneTwo => Self::TwoOne,
            Self::ZeroTwo => Self::TwoZero,
            Self::Unplayable => Self::Unplayable
        }
    }
}

type Matches = HashMap<(UserId, UserId), MatchResult>;
struct MatchMatrixSetup{
    threadname: String,
    shortname: String,
    users: Vec<LocalUser>,
}
struct MatchMatrix{
    thread: ChannelId,
    threadname: String,
    mainpost: MessageId,
    users: Vec<LocalUser>,
    results: Matches,
    disabled_fam: HashSet<UserId>,
}
struct Handler{
    setup_data: DashMap<GuildId, MatchMatrixSetup>,
    match_data: DashMap<GuildId, HashMap<String, MatchMatrix>>
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalUser{
    name: String,
    id: UserId,
    user: User,
}

fn render_grid(users: &[LocalUser], results: &Matches, disabled_fam: &HashSet<UserId>, header: &str) -> Result<String> {
    let mut message_str = header.to_string();
    message_str += "\n";
    for y in users{
        let mut wins = 0;
        let mut matches = 0;
        for x in users{
            let result = results.get(&(x.id, y.id)).context("Grid render failed: users not found in matrix")?;
            message_str.push_str(result.render());
            if [MatchResult::TwoZero, MatchResult::TwoOne].contains(result){
                wins += 1;
                matches += 1;
            }
            if [MatchResult::OneTwo, MatchResult::ZeroTwo].contains(result){
                matches += 1;
            }
            message_str.push(' ');
        }
        message_str.push_str(&format!("{}/{} {}{}\n", wins, matches, &y.name, if disabled_fam.contains(&y.id) {":no_bell:"} else {""}));
    }
    for user in users{
        let c = user.name.to_ascii_lowercase().chars().find(|x| x.is_ascii_alphanumeric());
        let id_square = if let Some(c) = c{
            if c.is_ascii_alphabetic(){
                format!(":regional_indicator_{c}:")
            } else {
                format!(":number_{c}:")
            }
        } else {
            String::from(":asterisk:")
        };
        message_str.push_str(&id_square);
        message_str.push(' ');
    }
    Ok(message_str)
}

fn lookup_userid(id: UserId, users: &[LocalUser]) -> Option<LocalUser>{
    for user in users{
        if user.id == id{
            return Some(user.clone())
        }
    }
    None
}

async fn localize_user(user: &User, ctx: &Context, guild: GuildId) -> Result<LocalUser>{
    let member = guild.member(ctx, user.id).await?;
    Ok(LocalUser{name: member.display_name().to_string(), id:user.id, user: user.clone()})
}
fn member_to_user(member: &Member) -> LocalUser{
    LocalUser{name: member.display_name().to_string(), id:member.user.id, user: member.user.clone()}
}

impl Handler{
    fn new() -> Self{
        Handler {setup_data: DashMap::new(), match_data: DashMap::new()}
    }

    fn begin(&self, command: &CommandInteraction) -> Result<String>{
        let options = &command.data.options();
        
        let guild = command.guild_id.context("guild not found in begin setup")?;
        if self.setup_data.contains_key(&guild){
            return Err(anyhow!("Starting new setup when already in the middle of setup"));
        }

        let Some(ResolvedOption {
            value: ResolvedValue::String(threadname), ..
        }) = options.get(0) else {return Err(anyhow!("name not found in begin setup"));};
        let threadname = threadname.to_string();

        let Some(ResolvedOption {
            value: ResolvedValue::String(shortname), ..
        }) = options.get(1) else {return Err(anyhow!("shortname not found in begin setup"));};
        let shortname = shortname.to_lowercase();
        //https://discord.com/developers/docs/interactions/application-commands#application-command-object-application-command-naming
        static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[-_\p{L}\p{N}\p{sc=Deva}\p{sc=Thai}]{1,32}$").unwrap());
        if !RE.is_match(&shortname) {return Err(anyhow!("invalid command name"))};

        self.setup_data.insert(guild, MatchMatrixSetup{threadname, shortname, users:Vec::new()});
        Ok("Success".to_string())
    }

    async fn add_users(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let options = &command.data.options();

        let guild = command.guild_id.context("guild not found in add name")?;
        if !self.setup_data.contains_key(&guild){
            return Err(anyhow!("Add name when not doing setup"));
        }
        let mut setup = self.setup_data.get_mut(&guild).context("Adding user without match setup")?;
        let mut users_added = 0;
        let mut extra_info = String::new();
        for current_user in 0..10{
            let Some(ResolvedOption {
                value: ResolvedValue::User(user, _), ..
            }) = options.get(current_user) else {continue;};
            let localized = localize_user(user, ctx, guild).await?;
            if lookup_userid(user.id, &setup.value().users).is_some(){
                extra_info += &localized.name;
                extra_info += " already included.\n";
                continue;
            }
            setup.value_mut().users.push(localized);
            users_added += 1;
        }

        Ok(format!("{}Added {} new players. Full list of {}: {:?}", extra_info, users_added, setup.value().users.len(), setup.value().users.iter().map(|x|&x.name).collect::<Vec<_>>()))
    }

    fn cancel(&self, command: &CommandInteraction) -> Result<String>{
        let guild = command.guild_id.context("guild not found in cancel")?;
        if !self.setup_data.contains_key(&guild){
            return Err(anyhow!("Cancel setup when not doing setup"));
        }
        self.setup_data.remove(&guild);
        Ok("Success".to_string())
    }

    async fn create(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let guild = command.guild_id.context("guild not found in create")?;
        let (_, setup) = self.setup_data.remove(&guild).context("Create called but setup data not found!")?;
        let thread_builder = CreateThread::new(&setup.threadname)
            .kind(ChannelType::PublicThread);
        let thread = command.channel_id.create_thread(&ctx.http, thread_builder).await?;

        let mut initial_message_str = String::new();
        for user in &setup.users{
            initial_message_str += &format!("<@{}> ", user.id);
        }
        thread.send_message(&ctx.http, CreateMessage::new()
            .allowed_mentions(CreateAllowedMentions::new().users(setup.users.iter().map(|x| &x.user)))
            .flags(MessageFlags::SUPPRESS_NOTIFICATIONS)
            .content(initial_message_str+" Report your results here using the command /"+&setup.shortname+" or /result"))
            .await?;

        let mut results = HashMap::new();
        for y in &setup.users{
            for x in &setup.users{
                let result = if x == y {MatchResult::Unplayable} else {MatchResult::NotPlayed};
                results.insert((x.id, y.id), result);
            }
        }
        let mainpost = thread.say(&ctx.http, render_grid(&setup.users, &results, &HashSet::new(), &setup.threadname)?).await?.id;

        thread.say(&ctx.http, ":cloud: match available\n:full_moon: match won 2-0\n:waning_gibbous_moon: match won 2-1\n\
            :waxing_crescent_moon: match lost 1-2\n:new_moon: match lost 0-2\n:black_small_square: cannot play yourself").await?;

        let mut match_vec = self.match_data.entry(guild).or_insert(HashMap::new());
        let matrix = MatchMatrix{thread: thread.id, threadname:setup.threadname, mainpost, users: setup.users, results, disabled_fam: HashSet::new()};
        match_vec.insert(setup.shortname, matrix);
        Self::reset_tournament_commands(ctx, &guild, &match_vec).await?;

        Ok("Success!".to_string())
    }

    async fn report_result_command(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let options = &command.data.options();
        let guild = command.guild_id.context("guild not found for result report")?;
        let Some(mut match_data_list) = self.match_data.get_mut(&guild) else { return Err(anyhow!("guild has no match matrices"));};
        
        let Some(ResolvedOption {
            value: ResolvedValue::String(result_str), ..
        }) = options.get(0) else {return Err(anyhow!("result not found in result report"));};
        let Some(ResolvedOption {
            value: ResolvedValue::String(opponent), ..
        }) = options.get(1) else {return Err(anyhow!("opponent not found in result report"));};
        let commandshortname = &command.data.name;
        for (shortname, matrix) in match_data_list.iter_mut(){
            if commandshortname == shortname{
                let opponent = lookup_userid(opponent.parse()?, &matrix.users).context("User not found")?;
                let player = lookup_userid(command.user.id, &matrix.users).context("User not found")?;
                return self.report_result_generic(ctx, matrix, &player, result_str, &opponent, &command.user).await;
            }
        }
        Err(anyhow!("Illegal command/name not found to report to"))
    }
    async fn report_result_any(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let options = &command.data.options();
        let guild = command.guild_id.context("guild not found for result report")?;
        let Some(mut match_data_list) = self.match_data.get_mut(&guild) else { return Err(anyhow!("guild has no match matrices"));};

        let Some(ResolvedOption {
            value: ResolvedValue::String(result_str), ..
        }) = options.get(0) else {return Err(anyhow!("result not found in result report"));};
        let Some(ResolvedOption {
            value: ResolvedValue::User(opponent, _), ..
        }) = options.get(1) else {return Err(anyhow!("opponent not found in result report"));};
        let player = match options.get(2) {
            Some(ResolvedOption {value: ResolvedValue::User(player, _), .. }) => player,
            _ => &command.user
        };
        for (_, matrix) in match_data_list.iter_mut(){
            if command.channel_id == matrix.thread{
                let player = lookup_userid(player.id, &matrix.users).context("User not found")?;
                let opponent = lookup_userid(opponent.id, &matrix.users).context("User not found")?;
                return self.report_result_generic(ctx, matrix, &player, result_str, &opponent, &command.user).await;
            }
        }
        Err(anyhow!("Attempted to report but results thread not found"))
    }
    async fn report_result_generic(&self, ctx: &Context, matrix: &mut MatchMatrix, player: &LocalUser, result_str: &str, opponent: &LocalUser, reporter_user: &User) -> Result<String>{
        if player.id == opponent.id {
            return Err(anyhow!("trying to report a match played against the same player"));
        }
        let result = MatchResult::get(result_str);
        let x = &mut matrix.results.get_mut(&(player.id, opponent.id)).context("match not found - bad user id?")?;
        **x = result.invert();
        let x2 = &mut matrix.results.get_mut(&(opponent.id, player.id)).context("reverse match not found - wtf?")?;
        **x2 = result;
        matrix.thread.say(&ctx.http, format!("{} reports {} {} {}", reporter_user, player.name, result_str, opponent.name)).await?;
        matrix.thread.message(&ctx.http, matrix.mainpost).await?.edit(&ctx.http, 
            EditMessage::new().content(render_grid(&matrix.users, &matrix.results, &matrix.disabled_fam, &matrix.threadname)?)).await?;
        Ok("Success".to_string())
    }

    async fn end(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let options = &command.data.options();
        let guild = command.guild_id.context("guild not found for end")?;
        let Some(mut match_data_list) = self.match_data.get_mut(&guild) else { return Err(anyhow!("guild has no match matrices"));};

        let Some(ResolvedOption {
            value: ResolvedValue::String(commandshortname), ..
        }) = options.get(0) else {return Err(anyhow!("name not found in end setup"));};

        let matchup = match_data_list.get(*commandshortname).context(format!("unable to find given name {} in match list", commandshortname))?;
        command.channel_id.say(&ctx.http, render_grid(&matchup.users, &matchup.results, &HashSet::new(), &matchup.threadname)?).await?;
        match_data_list.remove(*commandshortname);
        Self::reset_tournament_commands(ctx, &guild, &match_data_list).await?;
        Ok("Success".to_string())
    }

    async fn reset_tournament_commands(ctx: &Context, guild: &GuildId, tournaments: &HashMap<String, MatchMatrix>) -> Result<()>{
        //Returns the delta in number of tournament report commands
        let mut fam_user_options = CreateCommandOption::new(CommandOptionType::String, "tournament", "Ping which opponents").required(true);
        let mut findable_user_options = CreateCommandOption::new(CommandOptionType::String, "tournament", "Which tournaments to enable/disable Find A Match pings?").required(true);
        let mut ping_user_options = CreateCommandOption::new(CommandOptionType::String, "tournament", "Ping a tournament").required(true);
        let mut end_user_options = CreateCommandOption::new(CommandOptionType::String, "tournament", "The tournament to end").required(true);
        fam_user_options = fam_user_options.add_string_choice("All tournaments", "");
        findable_user_options = findable_user_options.add_string_choice("All tournaments", "");
        for (shortname, longname) in tournaments.iter().map(|(key, val)| (key, &val.threadname)){
            fam_user_options = fam_user_options.add_string_choice(longname, shortname);
            findable_user_options = findable_user_options.add_string_choice(longname, shortname);
            ping_user_options = ping_user_options.add_string_choice(longname, shortname);
            end_user_options = end_user_options.add_string_choice(longname, shortname);
        }
        let findable_enable_option = CreateCommandOption::new(CommandOptionType::Boolean, "findable", "Do you want to allow pings (true) or disable them (false)?").required(true);

        let mut commands = vec![
            CreateCommand::new("fam").description("Find A Match: Ping other players that you haven't played yet")
                .add_option(fam_user_options),
            CreateCommand::new("findable").description("Enable or disable pinging for Find A Match")
                .add_option(findable_user_options).add_option(findable_enable_option),
            CreateCommand::new("ping").description("Silent ping all players of a tournament")
                .default_member_permissions(Permissions::MODERATE_MEMBERS).add_option(ping_user_options),            
            CreateCommand::new("end").description("End a match matrix, posting final results in this channel")
                .default_member_permissions(Permissions::MODERATE_MEMBERS).add_option(end_user_options)
            ];

        for (shortname, tournament_matrix) in tournaments.iter(){
            let mut player_options = CreateCommandOption::new(CommandOptionType::String, "opponent", "Who was your opponent").required(true);
            for user in &tournament_matrix.users{
                player_options = player_options.add_string_choice(&user.name, user.id.to_string());
            }
            commands.push(CreateCommand::new(shortname)
            .description(format!("Submit result for {}", &tournament_matrix.threadname))
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "score", "What was the match score (you first)")
                .add_string_choice("2-0 (Win)", "2-0").add_string_choice("2-1 (Win)", "2-1")
                .add_string_choice("1-2 (Loss)", "1-2").add_string_choice("0-2 (Loss)", "0-2")
                .add_string_choice("0-0 (No result)", "0-0").required(true)
            ).add_option(player_options));
        }
        guild.set_commands(&ctx.http, commands).await?;
        Ok(())
    }

    async fn ping(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let options = &command.data.options();
        let guild = command.guild_id.context("guild not found for ping")?;
        let Some(match_data_list) = self.match_data.get(&guild) else { return Err(anyhow!("guild has no match matrices"));};

        let Some(ResolvedOption {
            value: ResolvedValue::String(commandshortname), ..
        }) = options.get(0) else {return Err(anyhow!("name not found in ping setup"));};

        let matchup = match_data_list.get(*commandshortname).context("unable to find given name in match list")?;
        let mut message_str = String::new();
        for user in &matchup.users{
            message_str = message_str+"<@"+&user.id.to_string()+"> ";
        }
        command.channel_id.send_message(&ctx.http, CreateMessage::new()
                .allowed_mentions(CreateAllowedMentions::new().users(matchup.users.iter().map(|x| &x.user)))
                .flags(MessageFlags::SUPPRESS_NOTIFICATIONS)
                .content(message_str))
                .await?;
        Ok("Success".to_string())
    }

    async fn fam_pings(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let options = &command.data.options();
        let guild = command.guild_id.context("guild not found for ping")?;
        let Some(match_data_list) = self.match_data.get(&guild) else { return Err(anyhow!("guild has no match matrices"));};
        let playerid = command.user.id;
        let mut mentions = HashSet::new();

        fn get_opponents(playerid: UserId, matrix: &MatchMatrix, mentions: &mut HashSet<UserId>) -> Option<String> {
            let _ = lookup_userid(playerid, &matrix.users)?;
            let mut message_str = String::new();
            let mut found_any = false;
            for opponent in &matrix.users{
                //Self is MatchResult::Unplayable so no need to special case it
                if matrix.results.get(&(playerid, opponent.id)) == Some(&MatchResult::NotPlayed) {
                    if matrix.disabled_fam.contains(&opponent.id){
                        message_str += &format!("{} ", opponent.name);
                    } else {
                        message_str += &format!("<@{}> ", opponent.id);
                        mentions.insert(opponent.id);
                    }
                    found_any = true;
                }
            }
            if !found_any {return Some(String::from("All matches complete!"));}
            Some(message_str)
        }

        let mut output = format!("<@{}> is trying to find a match to play, is anyone available?", playerid);
        let commandshortname = match options.get(0){
            Some(ResolvedOption {
                value: ResolvedValue::String(commandshortname), ..
            }) => commandshortname,
            None => "",
            _ => return Err(anyhow!("Bad command arguments"))
        };
        for (shortname, matrix) in match_data_list.iter(){
            if commandshortname.is_empty() || commandshortname == shortname{
                if let Some(opponents_string) = get_opponents(playerid, matrix, &mut mentions){
                    output += &format!("\n{}: {}", shortname, &opponents_string);
                }
            }
        }
        command.channel_id.send_message(&ctx.http, CreateMessage::new()
                .allowed_mentions(CreateAllowedMentions::new().users(mentions.iter()))
                .content(output))
                .await?;

        Ok("Success".to_string())
    }

    async fn findable(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let options = &command.data.options();
        let guild = command.guild_id.context("guild not found for ping")?;
        let Some(mut match_data_list) = self.match_data.get_mut(&guild) else { return Err(anyhow!("guild has no match matrices"));};
        let playerid = command.user.id;

        let Some(ResolvedOption {
            value: ResolvedValue::String(commandshortname), ..
        }) = options.get(0) else {return Err(anyhow!("command name argument missing"));};
        let Some(ResolvedOption {
            value: ResolvedValue::Boolean(enable), ..
        }) = options.get(1) else {return Err(anyhow!("enable argument missing"));};

        let mut count:i32 = 0;
        for (shortname, matrix) in match_data_list.iter_mut(){
            if commandshortname.is_empty() || commandshortname == shortname{
                if lookup_userid(playerid, &matrix.users).is_some(){
                    let result = if *enable {matrix.disabled_fam.remove(&playerid)} else {matrix.disabled_fam.insert(playerid)};
                    if result {
                        count += 1;
                        matrix.thread.message(&ctx.http, matrix.mainpost).await?.edit(&ctx.http, 
                            EditMessage::new().content(render_grid(&matrix.users, &matrix.results, &matrix.disabled_fam, &matrix.threadname)?)).await?;
                    }
                }
            }
        }
        Ok(format!("Success - {} findable statuses changed", count))
    }

    async fn reprocess(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let guild = command.guild_id.context("guild not found for reprocess")?;
        let messages = ctx.http.get_messages(command.channel_id, None, Some(100)).await?;
        let intro = &messages.get(messages.len()-1).context("intro message not found")?.content;
        let matrix_post = &messages.get(messages.len()-2).context("matrix message not found")?;

        //Read intro post for users and command name
        static RE_INTRO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*) Report your results here using the command /([^ ]+) or /result").unwrap());
        let content_match = RE_INTRO.captures(intro).context("intro message does not match expected")?;
        let user_str_list = content_match[1].split(" ");
        let mut user_list = Vec::new();
        let mut disabled_fam = HashSet::new();
        for str in user_str_list{
            static RE_USERID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<@(\d+)>").unwrap());
            if let Some(user_match) = RE_USERID.captures(str){
                let user = UserId::new(user_match[1].parse()?);
                user_list.push(member_to_user(&guild.member(&ctx.http, user).await?));
            }
        }
        let shortname = &content_match[2];

        //Read the matrix results
        let mut results = HashMap::new();
        static RE_MATCH_ICONS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":cloud:|:full_moon:|:waning_gibbous_moon:|:waxing_crescent_moon:|:new_moon:|:black_small_square:").unwrap());
        let mut matrix_match = RE_MATCH_ICONS.find_iter(&matrix_post.content);
        for y in &user_list{
            for x in &user_list{
                let result = MatchResult::get(matrix_match.next()
                    .context(format!("Unable to find match results matrix content: {} for {},{}", &matrix_post.content, x.name, y.name))?.as_str());
                results.insert((x.id, y.id), result);
            }
            if matrix_post.content.contains(&format!("{}:no_bell:", y.name)) {
                disabled_fam.insert(y.id);
            }
        }

        //final setup
        let user_count = user_list.len();
        let mainpost = matrix_post.id;
        let fullname = command.channel.as_ref().context("getting channel/thread")?.name.as_ref().context("getting channel/thread name")?;
        let matrix = MatchMatrix{thread: command.channel_id, threadname:fullname.to_string(), mainpost, users: user_list, results, disabled_fam};
        let render_result = render_grid(&matrix.users, &matrix.results, &matrix.disabled_fam, &matrix.threadname)?;
        if matrix_post.content != render_result {
            matrix.thread.message(&ctx.http, matrix.mainpost).await?.edit(&ctx.http,
                EditMessage::new().content(render_result)).await?;
        }
        let mut match_vec = self.match_data.entry(guild).or_insert(HashMap::new());
        match_vec.insert(shortname.to_string(), matrix);
        Self::reset_tournament_commands(ctx, &guild, &match_vec).await?;
        
        Ok(format!("Processed {} ({}) with {} users - currently running {} tournaments", fullname, shortname, user_count, match_vec.len()))
    }
}
#[async_trait]
impl EventHandler for Handler {
    /*async fn message(&self, ctx: Context, msg: Message) {
        // TODO: maybe support match reports in normal messages?
        if msg.content.eq("!setup commands") {
            println!("Setting up commands");
            let Some(guild) = msg.guild_id else {
                println!("guild not found for command setup");
                return;
            };
        }
    }*/
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            let response1 = command.create_response(&ctx.http, CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().ephemeral(true)
                .content("Processing"))).await;
            if let Err(why) = response1{
                println!("Cannot respond to slash command: {why}");
                return;
            }
            //println!("Received command interaction: {command:#?}");
            let result = match command.data.name.as_str() {
                "begin" => self.begin(&command),
                "add" => self.add_users(&ctx, &command).await,
                "create" => self.create(&ctx, &command).await,
                "cancel" => self.cancel(&command),
                "end" => self.end(&ctx, &command).await,
                "result" => self.report_result_any(&ctx, &command).await,
                "reprocess" => self.reprocess(&ctx, &command).await,
                "ping" => self.ping(&ctx, &command).await,
                "fam" => self.fam_pings(&ctx, &command).await,
                "findable" => self.findable(&ctx, &command).await,
                _ => self.report_result_command(&ctx, &command).await,
            };
            let response2 = command.edit_response(&ctx.http, EditInteractionResponse::new().content(
                match result{
                    Err(why) => why.to_string(), //{println!("{}", why.backtrace()); why.to_string()},
                    Ok(success_result) => success_result,
                })).await;
            if let Err(why) = response2{
                println!("Cannot edit slash command response: {why}");
            }
        }
    }

    async fn ready(&self, ctx: Context, _ready: Ready) {
        //Only need to do this once (or if I change the commands)
        /*let result = Command::set_global_commands(&ctx.http, vec![
            CreateCommand::new("begin").description("Begin setting up a new match matrix")
                .default_member_permissions(Permissions::MODERATE_MEMBERS)
                .add_option(CreateCommandOption::new(CommandOptionType::String, "title", "The name of the thread to make").required(true))
                .add_option(CreateCommandOption::new(CommandOptionType::String, "cmd", "The new command-name for results (lower case, no spaces)").required(true)),
            CreateCommand::new("add").description("Add user(s) for setup")
                .default_member_permissions(Permissions::MODERATE_MEMBERS)
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player", "First user to add").required(true))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player2", "Second user"))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player3", "Third user"))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player4", "Fourth user"))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player5", "Fifth user"))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player6", "Sixth user"))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player7", "Seventh user"))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player8", "Eighth user"))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player9", "Ninth user"))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player10", "Tenth user (if you need more, call the command again)")),
            CreateCommand::new("create").description("Create the match results matrix thread in this channel")
                .default_member_permissions(Permissions::MODERATE_MEMBERS),
            CreateCommand::new("cancel").description("Cancel the current match matrix setup")
                .default_member_permissions(Permissions::MODERATE_MEMBERS),
            CreateCommand::new("reprocess").description("Read this channel's matrix info into storage. Also resets unavailable report commands")
                .default_member_permissions(Permissions::MODERATE_MEMBERS),
            CreateCommand::new("result").description("Report a match result with arbitrary users for the current results thread")
                .add_option(CreateCommandOption::new(CommandOptionType::String, "score", "What was the match score")
                    .add_string_choice("2-0 (Win)", "2-0").add_string_choice("2-1 (Win)", "2-1")
                    .add_string_choice("1-2 (Loss)", "1-2").add_string_choice("0-2 (Loss)", "0-2")
                    .add_string_choice("0-0 (No result)", "0-0").required(true))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "opponent", "The second player in the match").required(true))
                .add_option(CreateCommandOption::new(CommandOptionType::User, "player", "Use an alternative first player in the match (otherwise assumed to be you)")),
            ]).await;
        if let Err(why) = result {
            println!("Error setting up global commands: {why:?}");
        }*/
    }
}

#[tokio::main]
async fn main() {
    // Login with a bot token from the environment
    let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");
    // Set gateway intents, which decides what events the bot will be notified about
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    // Create a new instance of the Client, logging in as a bot.
    let mut client =
        Client::builder(&token, intents).event_handler(Handler::new()).await.expect("Err creating client");

    // Start listening for events by starting a single shard
    if let Err(why) = client.start().await {
        println!("Client error: {why:?}");
    }
}
