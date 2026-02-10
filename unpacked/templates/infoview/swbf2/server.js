///////////////////////////////////////////////////
// Custom overrides for Star Wars Battlefront II //
///////////////////////////////////////////////////

render_player_team_list = function()
{
	var player_list_div = document.getElementById("player_team_list_id");
	if (player_list_div)
	{
		show_element("player_team_list_id", true);
	
		var strInnerHTML = "";

		var raw_serverinfo = { %raw_serverinfo% };
		var teamid = 0;
		while (teamid < raw_serverinfo['numteams'])
		{
			var team_name = raw_serverinfo['team_t' + teamid];
			if (team_name == null)
				break;

			// Start Box
			strInnerHTML += "<div id='player_list_box' class='player_box'>";
			
			// Box Title
			strInnerHTML += "<div id='player_list_box_title' class='box_title'>" + team_name + "</div>";
		
			// Box Details
			strInnerHTML += "<div class='box_details'>";
			strInnerHTML += "  <table class='player_list_table'>";
			strInnerHTML += "    <tr>";
			strInnerHTML += "      <th class='player_list_col_title_0'>%js:text_name%</th>";
			strInnerHTML += "      <td class='player_list_col_title_1'>%js:text_ping%</td>";
			strInnerHTML += "      <td class='player_list_col_title_1'>Hero Pts</td>";
			strInnerHTML += "      <td class='player_list_col_title_1'>Kills</td>";
			strInnerHTML += "      <td class='player_list_col_title_1'>Deaths</td>";
			strInnerHTML += "      <td class='player_list_col_title_1'>%js:text_score%</td>";
			strInnerHTML += "    </tr>";

			var i = 0;
			var count = 0;
			while (1)
			{
				var name = raw_serverinfo['player_' + i];
				if (!name)
					break;

				if (raw_serverinfo['team_' + i] == (teamid+1))
				{
					count++;

					var score = raw_serverinfo['score_' + i];
					var kills = raw_serverinfo['kills_' + i];
					if (!kills)
						kills = 0;
					var deaths = raw_serverinfo['deaths_' + i];
					var heropoints = raw_serverinfo['heropoints_' + i];
					if (!heropoints)
						heropoints = 0;
					var ping = raw_serverinfo['ping_' + i];
		
					strInnerHTML += "    <tr>";
					strInnerHTML += "      <th class='player_list_col_0'>" + colorizeName(name, "%js:servertype%") + "</th>";
					strInnerHTML += "      <td class='player_list_col_1'>" + ping + "</td>";
					strInnerHTML += "      <td class='player_list_col_1'>" + heropoints + "</td>";
					strInnerHTML += "      <td class='player_list_col_1'>" + kills + "</td>";
					strInnerHTML += "      <td class='player_list_col_1'>" + deaths + "</td>";
					strInnerHTML += "      <td class='player_list_col_1'>" + score + "</td>";
					strInnerHTML += "    </tr>";
				}
				i++;
			}

			var reinforcements = raw_serverinfo['team' + (teamid+1) + 'reinforcements'];
			// hack: we get 2 billion sometimes... (negative signed 32-bit int?)
			if (reinforcements > 1000000)
				reinforcements = null;
			var score = raw_serverinfo['score_t' + teamid];
			if (score > 1000000)
				score = null;
			
			if (score || reinforcements)
			{
				strInnerHTML += "    <tr>";
				strInnerHTML += "      <th class='total_frags' colspan='5'>%js:text_score%</th>";
				strInnerHTML += "      <td class='total_frags'>" + score + "</td>";
				strInnerHTML += "    </tr>";
				strInnerHTML += "    <tr>";
				strInnerHTML += "      <th class='total_frags' colspan='5'>Reinforcements</th>";
				strInnerHTML += "      <td class='total_frags'>" + reinforcements + "</td>";
				strInnerHTML += "    </tr>";
			}
			
			// End player table, box details
			strInnerHTML += "  </table>";
			strInnerHTML += "</div>";
			
			// End Box
			strInnerHTML += "</div>";
			
			teamid++;
		}
		
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

		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "AI Units Per Team";
		new_td = document.createElement("TD");
		new_td.innerText = raw_server_info['numai'];
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Min Players to Start";
		new_td = document.createElement("TD");
		new_td.innerText = raw_server_info['minplayers'];
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Teams";
		new_td = document.createElement("TD");
		if (raw_server_info['autoteam'] == 0)
			new_td.innerText = "Player Select";
		else
			new_td.innerText = "Auto Assign";
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);
		
		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Team Damage";
		new_td = document.createElement("TD");
		if (raw_server_info['teamdamage'] == 0)
			new_td.innerText = "Off";
		else
			new_td.innerText = "On";
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "AI Difficulty Level";
		new_td = document.createElement("TD");
		var strHowHard = "Easy";
		if (raw_server_info['aidifficulty'] == 2)
			strHowHard = "Normal";
		else if (raw_server_info['aidifficulty'] == 3)
			strHowHard = "Elite";
		new_td.innerText = strHowHard;
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Show Player Names";
		new_td = document.createElement("TD");
		if (raw_server_info['showplayernames'] == 0)
			new_td.innerText = "Off";
		else
			new_td.innerText = "On";
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Heroes";
		new_td = document.createElement("TD");
		if (raw_server_info['heroes'] == 0)
			new_td.innerText = "Off";
		else
			new_td.innerText = "On";
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Spawn Invincibility";
		new_td = document.createElement("TD");
		new_td.innerText = raw_server_info['invincibilitytime'] + " sec";
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

		new_tr = tbody.insertRow(-1); // append a row
		new_th = document.createElement("TH");
		new_th.className = "first_char_uppercase";
		new_th.innerText = "Version";
		new_td = document.createElement("TD");
		new_td.innerText = raw_server_info['gamever'];
		new_tr.appendChild(new_th);
		new_tr.appendChild(new_td);

	}

}




