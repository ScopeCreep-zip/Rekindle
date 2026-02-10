/////////////////////////////////
// Custom overrides for Quake4 //
/////////////////////////////////

render_player_team_list = function()
{
	var player_list_div = document.getElementById("player_team_list_id");
	if (player_list_div)
	{
		var raw_server_info = { %raw_serverinfo% };
		
		// There has to be at least one player for a player list
		if (!raw_server_info["player_0"])
			return;

		show_element("player_team_list_id", true);
			
		var strInnerHTML = "";

		// Start Box
		strInnerHTML += "<div id='player_list_box' class='player_box'>";
		
		// Box Title
		strInnerHTML += "<div id='player_list_box_title' class='box_title'>%js:text_playerlist%</div>";
		
		// Box Details
		strInnerHTML += "<div class='box_details'>";
		strInnerHTML += "  <table class='player_list_table'>";
		strInnerHTML += "    <tr>";
		strInnerHTML += "      <th class='player_list_col_title_0'>%js:text_name%</th>";
		strInnerHTML += "      <td class='player_list_col_title_1'>Clan</td>";
		strInnerHTML += "      <td class='player_list_col_title_1'>%js:text_ping%</td>";
		strInnerHTML += "      <td class='player_list_col_title_2'>Rate</td>";
		strInnerHTML += "    </tr>";

		var i = 0;
		while (1)
		{
			var name = raw_server_info['player_' + i];
			if (!name)
				break;

			var playerid = raw_server_info['playerid_' + i];
			var ping = raw_server_info['ping_' + i];
			var rate = raw_server_info['rate_' + i];
			var clan = raw_server_info['clan_' + i];

			/*
			if ((i % 2) == 1)
				document.write("<tr class='buddy'>");
			else
				document.write("<tr class='buddy' style=\"background-color: " + alt_bgcolor + "\">");
			*/

			var in_game_name = null;
			if (%has_game_stats_hash%)
			{
				var game_stats_hash = { %game_stats_hash% };
				if (game_stats_hash['name'])
					in_game_name = game_stats_hash['name'];
			}
			
			strInnerHTML += "    <tr>";

			// If its the selected user then highlight...
			if (in_game_name && in_game_name == name)
			{
				strInnerHTML += "      <th class='player_list_col_0_highlighted'>" + colorizeName(name, "%js:servertype%") + "</th>";
				strInnerHTML += "      <td class='player_list_col_1_highlighted'>" + colorizeName(clan, "%js:servertype%") + "</td>";
				strInnerHTML += "      <td class='player_list_col_1_highlighted'>" + ping + "</td>";
				strInnerHTML += "      <td class='player_list_col_2_highlighted'>" + rate + "</td>";
			}
			else
			{
				strInnerHTML += "      <th class='player_list_col_0'>" + colorizeName(name, "%js:servertype%") + "</th>";
				strInnerHTML += "      <td class='player_list_col_1'>" + colorizeName(clan, "%js:servertype%") + "</td>";
				strInnerHTML += "      <td class='player_list_col_1'>" + ping + "</td>";
				strInnerHTML += "      <td class='player_list_col_2'>" + rate + "</td>";
			}
			
			strInnerHTML += "    </tr>";

			i++;
		}

		strInnerHTML += "  </table>";
		strInnerHTML += "</div>";
		
		// End Box
		strInnerHTML += "</div>";

		player_list_div.innerHTML = strInnerHTML;

	}
}

render_server_extra = function()
{
	var tbody = document.getElementById("server_tbody_id");
	if (tbody)
	{
		var raw_server_info = { %raw_serverinfo% };
		var new_tr = null;
		var new_th = null;
		var new_td = null;

		// PUNKBUSTER				
		if (raw_server_info['sv_punkbuster'] == "1")
		{
			new_tr = tbody.insertRow(-1); // append a row
			new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "%js:text_punkbuster%";
			new_td = document.createElement("TD");
			new_td.innerHTML = "<img src='%media_template_folder%infoview/images/punkbuster.gif' alt='Punkbuster Enabled' />";
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}	

		// TIMELIMIT
		if (raw_server_info['si_timelimit'])
		{
			new_tr = tbody.insertRow(-1); // append a row
			new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "Time Limit";
			new_td = document.createElement("TD");
			new_td.innerText = raw_server_info['si_timelimit'];
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}
		
		// FRAGLIMIT
		if (raw_server_info['si_fraglimit'])
		{
			new_tr = tbody.insertRow(-1); // append a row
			new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "Frag Limit";
			new_td = document.createElement("TD");
			new_td.innerText = raw_server_info['si_fraglimit'];
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}
			
		// TEAM DAMAGE
		if (raw_server_info['si_teamdamage'])
		{
			new_tr = tbody.insertRow(-1); // append a row
			new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "Team Damage";
			new_td = document.createElement("TD");
			if (raw_server_info['si_teamdamage'] == "1")
				new_td.innerText = "Yes";
			else
				new_td.innerText = "No";
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}
						
		// AUTOBALANCE
		if (raw_server_info['si_autobalance'])
		{
			new_tr = tbody.insertRow(-1); // append a row
			new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "Auto Balance Teams";
			new_td = document.createElement("TD");
			if (raw_server_info['si_autobalance'] == "1")
				new_td.innerText = "Yes";
			else
				new_td.innerText = "No";
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}
						
		// PURE
		if (raw_server_info['si_pure'])
		{
			new_tr = tbody.insertRow(-1); // append a row
			new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "Pure";
			new_td = document.createElement("TD");
			if (raw_server_info['si_pure'] == "1")
				new_td.innerText = "Yes";
			else
				new_td.innerText = "No";
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}
						
		// SPECTATOSR
		if (raw_server_info['si_spectators'])
		{
			new_tr = tbody.insertRow(-1); // append a row
			new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "Allow Spectators";
			new_td = document.createElement("TD");
			if (raw_server_info['si_spectators'] == "1")
				new_td.innerText = "Yes";
			else
				new_td.innerText = "No";
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}				
	}

}