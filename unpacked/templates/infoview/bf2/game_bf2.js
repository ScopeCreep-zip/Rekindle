 %include scripts\AjaxRequest.js%
 %include scripts\linkify.js%

// Game Types array

var server_flag_url			= "%js:server_flag_url%";
var country_code			= "%js:country_code%";
var country_code_url		= "%js:country_code_url%";
var bShowRawServerInfo = false;

var game_types = { "gpm_cq":"Conquest", "gpm_ctf":"CTF" };

var retry_counts = { };
var all_pids = new Array();

var rank_strings = [ "Private", "Private First Class", "Lance Corporal", "Corporal", "Sergeant", "Staff Sergeant", "Gunnery Sergeant", "Master Sergeant", "First Sergeant", "Master Gunnery Sergeant", "Sergeant Major", "Sergeant Major of the Corps", "2nd Lieutenant", "1st Lieutenant", "Captain", "Major", "Lieutenant Colonel", "Colonel", "Brigadier General", "Major General", "Lieutenant General", "General" ];

function initialize()
{
  if (%user_ingame%) {
    var userinfo = document.getElementById("userinfo");
    var profilelink = document.getElementById("profile_link");
    if (userinfo) {
      userinfo.style.display = "";
      if (profilelink) {
		profilelink.setAttribute("href", "%js:profile_url%" + "%js:username%");
		profilelink.setAttribute("title", "%js:profile_url%" + "%js:username%");
      }
      if (%has_custom_status%) {
	userinfo.appendChild(createKeyValueRow("%js:text_status%", linkify("%js:custom_status%"), true));
      }
      if (%voice_hasip%) {
		userinfo.appendChild(createKeyValueRow("%js:text_voicechat%", "%js:voice_serverip%"));
      }
      if (%has_game_stats_hash%) {
		var game_stats_hash = { %game_stats_hash% };
		//for (var k in game_stats_hash) { userinfo.appendChild(createKeyValueRow(k, game_stats_hash[k], false)); }
		if (game_stats_hash['name']) {
			var playerQS = document.getElementById('playerQS');
			if (playerQS) {
				playerQS.style.display = "";
			}
		}
	  }
    }
  }

  if (%game_hasip%) {
    var gameinfo = document.getElementById("gameinfo");

    if (gameinfo) {
      var raw_serverinfo = { %raw_serverinfo% };
      
      
      gameinfo.style.display = "";
      if (%game_hasservername%) {
		var headingDiv = document.createElement("div");
		headingDiv.className = "heading";
		headingDiv.appendChild(document.createTextNode("%js:game_servername%"));
		gameinfo.appendChild(headingDiv);
      } else if (%game_hasquerystatus%) {
		var headingDiv = document.createElement("div");
		headingDiv.className = "heading";
		headingDiv.appendChild(document.createTextNode("%js:game_serverquerystatus%"));
		gameinfo.appendChild(headingDiv);
      }

      gameinfo.appendChild(createKeyValueRow("%js:text_serverip%", "<a title='Join Now!' href='xfire:join?game=bf2&server=%js:game_serverip%'>%js:game_serverip%</a>", true));
      if (%game_hasserverping%)
		gameinfo.appendChild(createKeyValueRow("%js:text_serverping%", "%js:game_serverping%"));
	
	  if (raw_serverinfo['mapname'])	
		gameinfo.appendChild(createKeyValueRow("Map", raw_serverinfo['mapname']));
	  if (%game_hasservergametype%) {
		if (game_types['%js:game_servergametype%']) {
			gameinfo.appendChild(createKeyValueRow("%js:text_servergametype%", game_types["%js:game_servergametype%"]));
		} else {
			gameinfo.appendChild(createKeyValueRow("%js:text_servergametype%", "%js:game_servergametype%"));
		}
	  }

	  if (raw_serverinfo['numplayers'] && raw_serverinfo['maxplayers'])
		gameinfo.appendChild(createKeyValueRow("Players", raw_serverinfo['numplayers'] + "/" + raw_serverinfo['maxplayers']));

      var options = "";
      
      if (country_code) {
		options += "&nbsp;<a href='" + country_code_url + "' target='_blank'><img src='" + server_flag_url + "' title='" + country_code + "' alt='' width='25' height='15' align='texttop' border='0'></a>";
      }
      
      if (raw_serverinfo['bf2_friendlyfire']) {
		options += '&nbsp;<img src="%media_template_folder%infoview/bf2/friendlyfire.gif" width="14" height="14" alt="Friendly Fire">';
      }
      if (raw_serverinfo['bf2_autobalanced']) {
		options += '&nbsp;<img src="%media_template_folder%infoview/bf2/autobalanced.gif" width="14" height="14" alt="Auto-Balanced">';
      }
      if (raw_serverinfo['bf2_ranked'] == "1") {
		options += '&nbsp;<img src="%media_template_folder%infoview/bf2/bars.gif" width="10" height="10" alt="Ranked">';
      }
      if (raw_serverinfo['bf2_anticheat'] == "1") {
		options += '&nbsp;<img src="%media_template_folder%infoview/images/punkbuster.gif" width="10" height="10" alt="Punkbuster Enabled">';
      }
      if (raw_serverinfo['bf2_voip'] == "1") {
		options += '&nbsp;<img src="%media_template_folder%infoview/images/headphones.gif" width="10" height="10" alt="Voice Chat">';
      }
      if (raw_serverinfo['password'] == "1") {
		options += '&nbsp;<img src="%media_template_folder%infoview/images/icon_lock.gif" width="10" height="10" alt="Password Protected">';
      }
      if (options != "")
		gameinfo.appendChild(createKeyValueRow("Info", options, true));

      // Now comes the meat-n-potatos
      // Let's create some team boxes, shall we?
      // We'll star by getting the team boxes
      var team1Div = document.getElementById("team1");
      var team2Div = document.getElementById("team2");

      if (raw_serverinfo['bf2_team1'] && raw_serverinfo['bf2_team2'] && team1Div && team2Div) {
		team1Div.style.display = "";
		team2Div.style.display = "";
	
		var team1Header = document.createElement("div");
		team1Header.className = "heading";
		team1Header.innerHTML = getFlagImage(raw_serverinfo.bf2_team1) + getTeamName(raw_serverinfo.bf2_team1);
		team1Div.appendChild(team1Header);
		team1Div.appendChild(createPlayerHeader());
	
		var team2Header = document.createElement("div");
		team2Header.className = "heading";
		team2Header.innerHTML = getFlagImage(raw_serverinfo.bf2_team2) + getTeamName(raw_serverinfo.bf2_team2);
		team2Div.appendChild(team2Header);
		team2Div.appendChild(createPlayerHeader());

		for (var p = 0; p < raw_serverinfo['numplayers']; p++) {
			var pid = raw_serverinfo['pid_' + p];
			all_pids.push(pid);
			var username = raw_serverinfo['player_' + p];
			var ping = raw_serverinfo['ping_' + p];
			var score = raw_serverinfo['score_' + p];
			if (pid && username && ping && score) {
				var user = createUserRow('pid_' + pid, 'row_' + pid, username, ping, score);
				var info = createInfoRow('info_' + pid);
				if (raw_serverinfo['team_' + p] == 1) {
					team1Div.appendChild(user);
					team1Div.appendChild(info);
				} else if (raw_serverinfo['team_' + p] == 2) {
					team2Div.appendChild(user);
					team2Div.appendChild(info);
				}
			}
		}
		if (raw_serverinfo['numplayers'] > 0) {
			var displayRanksButton = document.getElementById('display_ranks');
			if (displayRanksButton)
				displayRanksButton.style.display = "";
		}
      }
    }
    var debug = document.getElementById('raw_server_info_id');
    if (debug) {
		for (var k in raw_serverinfo) {
			debug.appendChild(createKeyValueRow(k, raw_serverinfo[k], false));
		}
    }
  }
}

function createKeyValueRow(key, value, useInnerHTML)
{
  var rowDiv = document.createElement("div");
  var keySpan = document.createElement("span");
  var valueSpan = document.createElement("span");
  rowDiv.className = "keyvalue";
  keySpan.className = "key";
  valueSpan.className = "value";
  keySpan.appendChild(document.createTextNode(key));
  if (useInnerHTML) {
    valueSpan.innerHTML = value;
  } else {
    valueSpan.appendChild(document.createTextNode(value));
  }
  rowDiv.appendChild(valueSpan);
  rowDiv.appendChild(keySpan);
  return rowDiv;
}

function createPlayerHeader()
{
  var rowDiv = document.createElement("div");
  var scoreSpan = document.createElement("span");
  var pingSpan = document.createElement("span");
  var nameSpan = document.createElement("span");
  var rankSpan = document.createElement("span");

  rowDiv.className = "keyvalue";
  scoreSpan.className = "score_header";
  pingSpan.className = "ping_header";
  nameSpan.className = "name_header";
  rankSpan.className = "rank_header";

  scoreSpan.appendChild(document.createTextNode("Score"));
  pingSpan.appendChild(document.createTextNode("Ping"));
  nameSpan.appendChild(document.createTextNode("Name"));
  rankSpan.appendChild(document.createTextNode("Rank"));

  rowDiv.appendChild(pingSpan);
  rowDiv.appendChild(scoreSpan);
  rowDiv.appendChild(nameSpan);
  rowDiv.appendChild(rankSpan);

  return rowDiv;
}

function createUserRow(pid, rid, username, ping, score)
{
  var rowDiv = document.createElement("div");
  var scoreSpan = document.createElement("span");
  var pingSpan = document.createElement("span");
  //var nameSpan = document.createElement("input");
  var nameSpan = document.createElement("span");
  var rankSpan = document.createElement("span");

  rowDiv.className = "keyvalue";
  rowDiv.id = rid;
  scoreSpan.className = "score";
  pingSpan.className = "ping";
  nameSpan.className = "name";
  //nameSpan.type = "text";
  //nameSpan.readOnly = true;
  rankSpan.className = "rank";
  rankSpan.id = pid;

  scoreSpan.appendChild(document.createTextNode(score));
  pingSpan.appendChild(document.createTextNode(ping));
  //nameSpan.value = username;
  nameSpan.innerHTML = username;
  rankSpan.innerHTML = "&nbsp;";

  rowDiv.appendChild(pingSpan);
  rowDiv.appendChild(scoreSpan);
  rowDiv.appendChild(nameSpan);
  rowDiv.appendChild(rankSpan);

  return rowDiv;
}

function createInfoRow(pid)
{
  var rowDiv = document.createElement("div");
  var fillerSpan1 = document.createElement("span");
  var fillerSpan2 = document.createElement("span");
  var rankSpan = document.createElement("span");
  
  rowDiv.className = "rank_box";
  fillerSpan1.className = "rank_filler";
  fillerSpan2.className = "rank_filler";
  rankSpan.className = "rank_desc";
  rankSpan.id = pid;

  rowDiv.style.display = "none";
  fillerSpan1.innerHTML = "&nbsp;";
  fillerSpan2.innerHTML = "&nbsp;";

  rowDiv.appendChild(fillerSpan1);
  rowDiv.appendChild(fillerSpan2);
  rowDiv.appendChild(rankSpan);

  return rowDiv;
}

function getTeamName(team_code)
{
  if (team_code == "MEC")
    return "Middle Eastern Coalition";
  if (team_code == "CH")
    return "People's Liberation Army";
  if (team_code == "US")
    return "United States Marine Corps";
  if (team_code == "EU")
	return "European Union";
  if (team_code == "SEAL")
	return "U.S. Navy SEALs";
  if (team_code == "SAS")
	return "British SAS";
  if (team_code == "Spetz")
	return "Russian Spetsnaz";
  if (team_code == "MECSF")
	return "M.E.C. Special Forces";
  if (team_code == "MEInsurgent")
	return "Middle East Insurgents";
  if (team_code == "Chinsurgent")
	return "Chinese Rebels";

  return team_code;
}

function getFlagImage(team_code)
{
  if (team_code == "MEC" || team_code == "MECSF")
    return "<img class=\"teamflag\" src=\"%media_template_folder%infoview/bf2/flag_mec.gif\" width=\"22\" height=\"14\" alt=\"" + getTeamName(team_code) + "\">";
  if (team_code == "CH")
    return "<img class=\"teamflag\" src=\"%media_template_folder%infoview/bf2/flag_ch.gif\" width=\"22\" height=\"14\" alt=\"" + getTeamName(team_code) + "\">";
  if (team_code == "US" || team_code == "SEAL")
    return "<img class=\"teamflag\" src=\"%media_template_folder%infoview/bf2/flag_us.gif\" width=\"22\" height=\"14\" alt=\"" + getTeamName(team_code) + "\">";
  if (team_code == "SAS")
    return "<img class=\"teamflag\" src=\"%media_template_folder%infoview/bf2/flag_uk.gif\" width=\"22\" height=\"14\" alt=\"" + getTeamName(team_code) + "\">";
  if (team_code == "Spetz")
    return "<img class=\"teamflag\" src=\"%media_template_folder%infoview/bf2/flag_ru.gif\" width=\"22\" height=\"14\" alt=\"" + getTeamName(team_code) + "\">";
  if (team_code == "Chinsurgent")
    return "<img class=\"teamflag\" src=\"%media_template_folder%infoview/bf2/flag_rebels.gif\" width=\"22\" height=\"14\" alt=\"" + getTeamName(team_code) + "\">";
  if (team_code == "MEInsurgent")
    return "<img class=\"teamflag\" src=\"%media_template_folder%infoview/bf2/flag_insurgents.gif\" width=\"22\" height=\"14\" alt=\"" + getTeamName(team_code) + "\">";
  if (team_code == "EU")
    return "<img class=\"teamflag\" src=\"%media_template_folder%infoview/bf2/flag_eu.gif\" width=\"22\" height=\"14\" alt=\"" + getTeamName(team_code) + "\">";

  return '';
}

function clearRetries()
{
  for (var p in retry_counts) {
    retry_counts[p] = 0;
  }
}

var currently_fetching = false;

function fetchRanks(profileid_array)
{
  if (currently_fetching)
    return;

  currently_fetching = true;
  var queryStr = "";
  for (var p = 0; p < profileid_array.length; p++) {
    if (retry_counts['pid_' + profileid_array[p]] == null ||
	retry_counts['pid_' + profileid_array[p]] < 5) {
      if (queryStr.length) {
	queryStr += "&profileid[]=" + escape(profileid_array[p]);
      } else {
	queryStr += "profileid[]=" + escape(profileid_array[p]);
      }
    }
  }
  if (queryStr != "") {
    AjaxRequest.post(
		     {
		       'url':'%scripting_host%/client/games_stats/bf2.php',
			 'queryString':queryStr,
			 'timeout':10000,
			 'onSuccess':
		       function (req)
			 {
			   currently_fetching = false;
			   var error = req.responseXML.getElementsByTagName('error');
			   if (error.length > 0) {
			     alert(error[0].firstChild.nodeValue);
			   } else {
			     fillOutRanks(req.responseXML);
			   }
			 },
			 'onError':
		       function (req)
			 {
			   currently_fetching = false;
			   alert("There was an error connecting to the server, please try again in a few minutes.");
			 },
			 'onTimeout':
		       function (req)
			 {
			   currently_fetching = false;
			   alert("The server took too long to respond, please try again in a few minutes.");
			 }
		     }
		     );
  }
}

function ToggleRawServerInfo(link_element)
{
	var raw_element = document.getElementById("raw_server_info_id");
	if (raw_element)
	{
		if (bShowRawServerInfo == false)
		{
			raw_element.style.display = 'block';
			bShowRawServerInfo = true;
			link_element.innerHTML = "%text_hide_rawserverinfo%";
		}
		else
		{
			raw_element.style.display = 'none';
			bShowRawServerInfo = false;
			link_element.innerHTML = "%text_display_rawserverinfo%";
		}
	}
}

function fillOutRanks(xml)
{
  var players = xml.getElementsByTagName("player");
  for (var i = 0; i < players.length; i++) {
    var pid = players[i].getAttribute("userid");
    var rank = players[i].getAttribute("rank");
    var retry = players[i].getAttribute("retry");
    if (pid) {
      var img = document.getElementById("pid_" + pid);
      var desc = document.getElementById("info_" + pid);

      if (img && desc && rank) {
	img.innerHTML = "<img src='http://media.xfire.com/v4/client/bf2ranks/rank_" + rank + ".gif' width='32' height='32'>";
	desc.innerHTML = rank_strings[rank];
	desc.parentElement.style.display = "";
      }
      if (retry) {
	if (retry_counts['pid_' + pid])
	  retry_counts['pid_' + pid]++;
	else
	  retry_counts['pid_' + pid] = 1;

	setTimeout("fetchRanks(['" + pid + "']);", retry);
      }
    }
  }
}

var playerQSEverShown = false;
function expandPlayerQS()
{
	var playerQSExpand = document.getElementById('playerQSExpand');
	var playerQSContract = document.getElementById('playerQSContract');
	var playerQSFetching = document.getElementById('playerQSFetching');
	var playerQSData = document.getElementById('playerQSData');
	var playerQSRank = document.getElementById('playerQSRank');
	if (playerQSExpand && playerQSContract && playerQSFetching && playerQSData && playerQSRank) {
		playerQSExpand.style.display = "none";
		playerQSContract.style.display = "";
		if (!playerQSEverShown) {
			playerQSEverShown = true;
			var game_stats_hash = { %game_stats_hash% };
			if (game_stats_hash['name']) {
				fetchPlayerQS(game_stats_hash['name']);
			}
		} else {
			playerQSData.style.display = "";
		}
	}
}

function contractPlayerQS()
{
	var playerQSExpand = document.getElementById('playerQSExpand');
	var playerQSContract = document.getElementById('playerQSContract');
	var playerQSFetching = document.getElementById('playerQSFetching');
	var playerQSData = document.getElementById('playerQSData');
	if (playerQSExpand && playerQSContract && playerQSFetching && playerQSData) {
		playerQSExpand.style.display = "";
		playerQSContract.style.display = "none";
		playerQSFetching.style.display = "none";
		playerQSData.style.display = "none";
	}
}
function swapStr(obj, str1, str2)
{
	if (obj.innerHTML == str1)
	{
		obj.innerHTML = str2;
	}
	else
	{
		obj.innerHTML = str1;
	}
}

function timeString(seconds)
{
	var minutes = Math.round(seconds / 60);
	var hours = Math.round(minutes / 60);
	var days = Math.round(hours / 24);
	var hstr = "";
	var dstr = "";
	if (days > 0)
		dstr = days + " days, " + (hours%24) + " hours";
	else if (hours > 0)
		dstr = hours + " hours, " + (minutes%60) + " minutes";
	else
		dstr = minutes + " minutes, " + (seconds%60) + " seconds";
	hstr = hours + " hours, " + (minutes%60) + " minutes";
	return "<span style='cursor:pointer' onclick='swapStr(this, \"" + dstr + "\", \"" + hstr + "\");'>" + dstr + "</span>";
}

var playerQSCount = 0;
function fetchPlayerQS(username)
{
	if (playerQSCount < 5) {
		playerQSCount++;
		playerQSFetching.style.display = "";
		AjaxRequest.post(
			{
				'url':'%scripting_host%/client/games_stats/bf2qs.php',
			        'queryString':"player=" + escape(username),
				'timeout':10000,
				'onSuccess':
					function (req)
					{
						var error = req.responseXML.getElementsByTagName('error');
						if (error.length > 0) {
							alert(error[0].firstChild.nodeValue);
							contractPlayerQS();
						} else {
							var players = req.responseXML.getElementsByTagName('player');
							if (players && players.length > 0) {
								var retry = players[0].getAttribute("retry");
								if (retry) {
									setTimeout("fetchPlayerQS('" + username + "');", retry);
								}
								var rank = players[0].getAttribute("rank");
								var score = players[0].getAttribute("score");
								var wins = players[0].getAttribute("wins");
								var losses = players[0].getAttribute("losses");
								var totalTime = players[0].getAttribute("totalTime");
								if (rank && score && wins && losses && totalTime) {
	   								playerQSFetching.style.display = "none";
									while (playerQSData.firstChild) {
									  playerQSData.removeChild(playerQSData.firstChild);
									}
									playerQSData.style.display = "";
									playerQSData.appendChild(createKeyValueRow("Rank", rank_strings[rank], false));
									playerQSData.appendChild(createKeyValueRow("Total Score", score, false));
									playerQSData.appendChild(createKeyValueRow("Wins/Losses", wins + "/" + losses, false));
									playerQSData.appendChild(createKeyValueRow("Total Time Spent", timeString(totalTime), true));
									playerQSRank.src = "http://media.xfire.com/v4/client/bf2ranks/rank_" + rank + ".gif";
									playerQSRank.style.display = "";
									var pid = players[0].getAttribute("userid");
									if (pid) {
										var playerRow = document.getElementById('row_' + pid);
										if (playerRow) {
											playerRow.className = 'selected_keyvalue';
										}
									}
								} else {
									contractPlayerQS();
								}
							}
						}
					},
				'onError':
					function (req)
					{
						alert("There was an error connecting to the server, please try again in a few minutes.");
						playerQSEverShown = false;
						contractPlayerQS();
					},
				'onTimeout':
					function (req)
					{
						alert("The server took too long to respond, please try again in a few minutes.");
						playerQSEverShown = false;
						contractPlayerQS();
					}
			}
		);
	}
}
