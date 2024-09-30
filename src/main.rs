use std::env;

use serenity::all::*;
use anyhow::{Result, Context as _, anyhow}; //overrides serenity Result

use std::collections::HashMap;
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
            "2-0"|":full_moon:" => return Self::TwoZero,
            "2-1"|":waning_gibbous_moon:" => return Self::TwoOne,
            "1-2"|":waxing_crescent_moon:" => return Self::OneTwo,
            "0-2"|":new_moon:" => return Self::ZeroTwo,
            "0-0"|":cloud:" => return Self::NotPlayed,
            ":black_small_square:" | _ => return Self::Unplayable,
        }
    }
    fn render(&self) -> &str{
        match self{
            Self::NotPlayed => return &":cloud:",
            Self::TwoZero => return &":full_moon:",
            Self::TwoOne => return &":waning_gibbous_moon:",
            Self::OneTwo => return &":waxing_crescent_moon:",
            Self::ZeroTwo => return &":new_moon:",
            Self::Unplayable => return &":black_small_square:"
        }
    }
    fn invert(&self) -> Self{
        match self{
            Self::NotPlayed => return Self::NotPlayed,
            Self::TwoZero => return Self::ZeroTwo,
            Self::TwoOne => return Self::OneTwo,
            Self::OneTwo => return Self::TwoOne,
            Self::ZeroTwo => return Self::TwoZero,
            Self::Unplayable => return Self::Unplayable
        }
    }
}

type Matches = HashMap<(UserId, UserId), MatchResult>;
struct MatchMatrixSetup{
    threadname: String,
    shortname: String,
    users: Vec<LocalUser>
}
struct MatchMatrix{
    thread: ChannelId,
    threadname: String,
    mainpost: MessageId,
    users: Vec<LocalUser>,
    results: Matches
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

fn render_grid(users: &[LocalUser], results: &Matches, header: &str) -> Result<String> {
    let mut message_str = header.to_string();
    message_str+="\n";
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
        message_str.push_str(&format!("{}/{} {}\n", wins, matches, &y.name));
    }
    for user in users{
        let c = user.name.to_ascii_lowercase().chars().filter(|x| x.is_ascii_alphanumeric()).next();
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
    return Ok(message_str);
}

fn lookup_userid<'a>(id: UserId, users: &'a [LocalUser]) -> Option<LocalUser>{
    for user in users{
        if user.id == id{
            return Some(user.clone())
        }
    }
    None
}

async fn localize_user<'a>(user: &User, ctx: &Context, guild: GuildId) -> Result<LocalUser>{
    let member = guild.member(ctx, user.id).await?;
    Ok(LocalUser{name: member.display_name().to_string(), id:user.id, user: user.clone()})
}
fn member_to_user<'a>(member: &Member) -> LocalUser{
    return LocalUser{name: member.display_name().to_string(), id:member.user.id, user: member.user.clone()}
}

impl Handler{
    fn new() -> Self{
        return Handler {setup_data: DashMap::new(), match_data: DashMap::new()};
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

    async fn register_match_command(ctx: &Context, guild: &GuildId, users: &[LocalUser], fullname: &str, shortname: &str) -> Result<()>{
        let mut player_options = CreateCommandOption::new(CommandOptionType::String, "opponent", "Who was your opponent").required(true);
        for user in users{
            player_options = player_options.add_string_choice(&user.name, user.id.to_string());
        }
        guild.create_command(&ctx.http, CreateCommand::new(shortname)
        .description(format!("Submit result for {}", &fullname))
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "score", "What was the match score (you first)")
            .add_string_choice("2-0 (Win)", "2-0").add_string_choice("2-1 (Win)", "2-1")
            .add_string_choice("1-2 (Loss)", "1-2").add_string_choice("0-2 (Loss)", "0-2")
            .add_string_choice("0-0 (No result)", "0-0").required(true)
        ).add_option(player_options)).await?;
        Ok(())
    }

    async fn create(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let guild = command.guild_id.context("guild not found in create")?;
        let setup = self.setup_data.get(&guild).context("Create called when not doing setup!")?;
        let thread_builder = CreateThread::new(&setup.threadname)
            .kind(ChannelType::PublicThread);
        let thread = command.channel_id.create_thread(&ctx.http, thread_builder).await?;

        let mut initial_message_str = String::new();
        for user in &setup.users{
            initial_message_str = initial_message_str+"<@"+&user.id.to_string()+"> ";
        }
        thread.send_message(&ctx.http, CreateMessage::new()
            .allowed_mentions(CreateAllowedMentions::new().users(setup.users.iter().map(|x| &x.user).into_iter()))
            .content(initial_message_str+" Report your results here using the command /"+&setup.shortname+" or /result"))
            .await?;

        let mut results = HashMap::new();
        for y in &setup.users{
            for x in &setup.users{
                let result = if x == y {MatchResult::Unplayable} else {MatchResult::NotPlayed};
                results.insert((x.id, y.id), result);
            }
        }
        let mainpost = thread.say(&ctx.http, render_grid(&setup.users, &results, &setup.threadname)?).await?.id;
        Self::register_match_command(ctx, &guild, &setup.users, &setup.threadname, &setup.shortname).await?;

        thread.say(&ctx.http, ":cloud: match available\n:full_moon: match won 2-0\n:waning_gibbous_moon: match won 2-1\n\
            :waxing_crescent_moon: match lost 1-2\n:new_moon: match lost 0-2\n:black_small_square: cannot play yourself").await?;

        drop(setup); //Prevent deadlock
        let (_, setup) = self.setup_data.remove(&guild).context("setup data not found to remove!")?;
        let mut match_vec = self.match_data.entry(guild).or_insert(HashMap::new());
        let matrix = MatchMatrix{thread: thread.id, threadname:setup.threadname, mainpost, users: setup.users, results};
        match_vec.insert(setup.shortname, matrix);

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
        return Err(anyhow!("Illegal command/name not found to report to"));
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
        return Err(anyhow!("Attempted to report but results thread not found"));
    }
    async fn report_result_generic(&self, ctx: &Context, matrix: &mut MatchMatrix, player: &LocalUser, result_str: &str, opponent: &LocalUser, reporter_user: &User) -> Result<String>{
        if player.id == opponent.id {
            return Err(anyhow!("trying to report a match played against the same player"));
        }
        let result = MatchResult::get(result_str);
        let ref mut x = matrix.results.get_mut(&(player.id, opponent.id)).context("match not found - bad user id?")?;
        **x = result.invert();
        let ref mut x = matrix.results.get_mut(&(opponent.id, player.id)).context("reverse match not found - wtf?")?;
        **x = result;
        matrix.thread.say(&ctx.http, format!("{} reports {} {} {}", reporter_user, player.name, result_str, opponent.name)).await?;
        matrix.thread.message(&ctx.http, matrix.mainpost).await?.edit(&ctx.http, 
            EditMessage::new().content(render_grid(&matrix.users, &matrix.results, &matrix.threadname)?)).await?;
        return Ok("Success".to_string());
    }

    async fn end(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let options = &command.data.options();
        let guild = command.guild_id.context("guild not found for end")?;
        let Some(mut match_data_list) = self.match_data.get_mut(&guild) else { return Err(anyhow!("guild has no match matrices"));};

        let commands_list = guild.get_commands(&ctx.http).await?;
        let Some(ResolvedOption {
            value: ResolvedValue::String(commandshortname), ..
        }) = options.get(0) else {return Err(anyhow!("name not found in begin setup"));};

        let matchup = match_data_list.get(*commandshortname).context("unable to find given name in match list")?;
        for slashcommand in &commands_list {
            if &slashcommand.name == commandshortname{
                guild.delete_command(&ctx.http, slashcommand.id).await?;
                command.channel_id.say(&ctx.http, render_grid(&matchup.users, &matchup.results, &matchup.threadname)?).await?;
                match_data_list.remove(*commandshortname);
                return Ok("Success".to_string());
            }
        }
        Err(anyhow!("Unable to find matchup in data"))
    }

    async fn reprocess(&self, ctx: &Context, command: &CommandInteraction) -> Result<String>{
        let guild = command.guild_id.context("guild not found for reprocess")?;
        let mut match_vec = self.match_data.entry(guild).or_insert(HashMap::new());
        if match_vec.len() == 0{
            guild.set_commands(&ctx.http, vec![]).await?;
        }
        let messages = command.channel_id.messages(&ctx.http, GetMessages::new().limit(100)).await?;
        let intro = &messages.get(messages.len()-1).context("intro message not found")?.content;
        let matrix_post = &messages.get(messages.len()-2).context("matrix message not found")?;

        //Read intro post for users and command name
        static RE_INTRO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*) Report your results here using the command /([^ ]+) or /result").unwrap());
        let content_match = RE_INTRO.captures(intro).context("intro message does not match expected")?;
        let user_str_list = content_match[1].split(" ");
        let mut user_list = Vec::new();
        for str in user_str_list{
            static RE_USERID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<@(\d+)>").unwrap());
            if let Some(user_match) = RE_USERID.captures(str){
                user_list.push(member_to_user(&guild.member(&ctx.http, UserId::new(user_match[1].parse()?)).await?));
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
        }

        //final setup
        let user_count = user_list.len();
        let mainpost = matrix_post.id;
        let tmp = &command.channel;
        let fullname = &tmp.as_ref().context("getting channel/thread")?.name.as_ref().context("getting channel/thread name")?;
        Self::register_match_command(ctx, &guild, &user_list, fullname, shortname).await?;

        let matrix = MatchMatrix{thread: command.channel_id, threadname:fullname.to_string(), mainpost, users: user_list, results};
        if matrix_post.content != render_grid(&matrix.users, &matrix.results, &matrix.threadname)?{
            matrix.thread.message(&ctx.http, matrix.mainpost).await?.edit(&ctx.http,
                EditMessage::new().content(render_grid(&matrix.users, &matrix.results, &matrix.threadname)?)).await?;
        }
        match_vec.insert(shortname.to_string(), matrix);
        
        Ok(format!("Processed {} ({}) with {} users", fullname, shortname, user_count))
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
            //println!("Received command interaction: {command:#?}");
            let result = match command.data.name.as_str() {
                "begin" => self.begin(&command),
                "add" => self.add_users(&ctx, &command).await,
                "create" => self.create(&ctx, &command).await,
                "cancel" => self.cancel(&command),
                "end" => self.end(&ctx, &command).await,
                "result" => self.report_result_any(&ctx, &command).await,
                "reprocess" => self.reprocess(&ctx, &command).await,
                _ => self.report_result_command(&ctx, &command).await,
            };
            let response = command.create_response(&ctx.http, CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().ephemeral(true)
            .content(
                match result{
                    Err(why) => why.to_string(),
                    Ok(success_result) => success_result,
                }))).await;
            if let Err(why2) = response{
                println!("Cannot respond to slash command: {why2}");
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
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
            CreateCommand::new("end").description("End a match matrix, posting final results in this channel")
                .default_member_permissions(Permissions::MODERATE_MEMBERS)
                .add_option(CreateCommandOption::new(CommandOptionType::String, "cmd", "The command-name of the tournament to end").required(true)),
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
