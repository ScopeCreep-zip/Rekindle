render_game_stats_hash = function()
{
	if (%has_game_stats_hash%)
	{
		var game_stats_hash = { %game_stats_hash% };

		var tbody = document.getElementById("server_tbody_id");
		if (tbody)
		{
			var tbody_rows = tbody.rows;
			var nInsertionRow = -1; // appends new rows
			if (tbody_rows)
			{
				// want to insert new rows prior to the rcon row
				for (var rowit = 0; rowit < tbody_rows.length; ++rowit)
				{
					var tr_element = tbody_rows.item(rowit);
					if (tr_element && tr_element.id == "rcon_row")
					{
						nInsertionRow = rowit;
						break;
					}
				}
			}
			if (game_stats_hash['xp'] && game_stats_hash['xp'] != 0 && game_stats_hash['level'])
			{
			   
				var level = Math.floor(game_stats_hash['level']);
			  
				var xpinfo = game_stats_hash['xp'].split(";");
				var bar_width = 150;
				var lux_faction = ((xpinfo[3] / 10000) * bar_width);
				var kurz_faction = ((xpinfo[2] / 10000) * bar_width);
				var balth_faction = ((xpinfo[1] / game_stats_hash['fact-balth-cap']) * bar_width);
				if (xpinfo[0] > 154000)
					level = level + 1;
				if (xpinfo[0] > 168000)
					level = level + 1;
				if (xpinfo[0] > 182600)
					level = level + 1;
				var xp_max = (level * 600 + 1400);
				var xp_min = (2000 * (level - 1)) + (300 * (level - 1) * (level - 2));
				var xp_value = (xpinfo[0] - xp_min);
				if (level > 22)
				{
					xp_max = 15000;
					xp_value = (xpinfo[0] - xp_min) % 15000;
				}
				if (xp_value > xp_max)
					xp_value = xp_max;

				var xp_bar = ((xp_value / xp_max) * bar_width);

				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.className = "bar_text"; new_td.innerText = "Luxon Faction";	new_tr.appendChild(new_td);
				var new_td = document.createElement("TD");
				new_td.innerHTML = "<div class='faction_border' style='width: " + bar_width + "px'><div class='faction_bar' style='width: " + (lux_faction - 4) + "px;'></div><div class='faction_bar_end' style='width: " + lux_faction + "px;'></div><div class='xp_text' style='width: " + bar_width + "px'>" + xpinfo[3] + " / 10000</div></div>";
				new_tr.appendChild(new_td);
        
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.className = "bar_text"; new_td.innerText = "Kurzick Faction";	new_tr.appendChild(new_td);
				var new_td = document.createElement("TD");
				new_td.innerHTML = "<div class='faction_border' style='width: " + bar_width + "px'><div class='faction_bar' style='width: " + (kurz_faction - 4) + "px;'></div><div class='faction_bar_end' style='width: " + kurz_faction + "px;'></div><div class='xp_text' style='width: " + bar_width + "px'>" + xpinfo[2] + " / 10000</div></div>";
				new_tr.appendChild(new_td);

				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.className = "bar_text"; new_td.innerText = "Balthazar Faction";	new_tr.appendChild(new_td);
				var new_td = document.createElement("TD");
				new_td.innerHTML = "<div class='faction_border' style='width: " + bar_width + "px'><div class='faction_bar' style='width: " + (balth_faction - 4) + "px;'></div><div class='faction_bar_end' style='width: " + balth_faction + "px;'></div><div class='xp_text' style='width: " + bar_width + "px'>" + xpinfo[1] + " / " + game_stats_hash['fact-balth-cap'] + "</div></div>";
				new_tr.appendChild(new_td);
        
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.className = "bar_text"; new_td.innerText = "XP";	new_tr.appendChild(new_td);
				var new_td = document.createElement("TD");
				new_td.innerHTML = "<div class='xp_border' style='width: " + bar_width + "px'><div class='xp_bar' style='width: " + (xp_bar - 4) + "px;'></div><div class='xp_bar_end' style='width: " + xp_bar + "px;'></div><div class='xp_text' style='width: " + bar_width + "px'>" + xpinfo[0] + "</div></div>";
				new_tr.appendChild(new_td);
  				
			}
			
			if (game_stats_hash['skill-1-id'] != 0)
			{
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				var gw_template_folder = '%media_template_folder%infoview/gw/';
				var skillarray = game_stats_hash['skill-1-id'].split(';');
				new_td.innerHTML = "<a href='http://wiki.guildwars.com/wiki/" + game_stats_hash['skill-1-name'] + "'><img border='0' src='" + gw_template_folder + skillarray[0] + ".jpg' title='" + game_stats_hash['skill-1-name'] + "' height='30' /></a>";
				new_td.innerHTML += "<a href='http://wiki.guildwars.com/wiki/" + game_stats_hash['skill-2-name'] + "'><img border='0' src='" + gw_template_folder + skillarray[1] + ".jpg' title='" + game_stats_hash['skill-2-name'] + "' height='30' /></a>";
				new_td.innerHTML += "<a href='http://wiki.guildwars.com/wiki/" + game_stats_hash['skill-3-name'] + "'><img border='0' src='" + gw_template_folder + skillarray[2] + ".jpg' title='" + game_stats_hash['skill-3-name'] + "' height='30' /></a>";
				new_td.innerHTML += "<a href='http://wiki.guildwars.com/wiki/" + game_stats_hash['skill-4-name'] + "'><img border='0' src='" + gw_template_folder + skillarray[3] + ".jpg' title='" + game_stats_hash['skill-4-name'] + "' height='30' /></a>";
				new_td.innerHTML += "<a href='http://wiki.guildwars.com/wiki/" + game_stats_hash['skill-5-name'] + "'><img border='0' src='" + gw_template_folder + skillarray[4] + ".jpg' title='" + game_stats_hash['skill-5-name'] + "' height='30' /></a>";
				new_td.innerHTML += "<a href='http://wiki.guildwars.com/wiki/" + game_stats_hash['skill-6-name'] + "'><img border='0' src='" + gw_template_folder + skillarray[5] + ".jpg' title='" + game_stats_hash['skill-6-name'] + "' height='30' /></a>";
				new_td.innerHTML += "<a href='http://wiki.guildwars.com/wiki/" + game_stats_hash['skill-7-name'] + "'><img border='0' src='" + gw_template_folder + skillarray[6] + ".jpg' title='" + game_stats_hash['skill-7-name'] + "' height='30' /></a>";
				new_td.innerHTML += "<a href='http://wiki.guildwars.com/wiki/" + game_stats_hash['skill-8-name'] + "'><img border='0' src='" + gw_template_folder + skillarray[7] + ".jpg' title='" + game_stats_hash['skill-8-name'] + "' height='30' /></a>";
				new_td.colSpan = 2;
				new_tr.appendChild(new_td);
			}
  		
			if (game_stats_hash['title'])
			{
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.colSpan = 2;
				new_td.innerText = "Displayed Title: " + game_stats_hash['title'];
				new_tr.appendChild(new_td);
			}

			if (game_stats_hash['loc'])
			{
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.colSpan = 2;
				new_td.className = "location";
				new_td.innerText = "Currently in: " + game_stats_hash['loc'];
				if (game_stats_hash['dist'] != 0)
				{
					switch(game_stats_hash['region'])
					{
					  case "0":
						  new_td.innerText += " - America - ";
						  break;
					  case "1":
						  new_td.innerText += " - Asia - ";
						  break;
					  case "2":
						  new_td.innerText += " - Europe - ";
						  break;
					  case "14":
						  new_td.innerText += " - International - ";
						  break;
					}
					new_td.innerText += "District " + game_stats_hash['dist'];
				}
				new_tr.appendChild(new_td);
			}
			
			if (game_stats_hash['hardmode'] != 0)
			{
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.colSpan = 2;
				new_td.innerText = "Hard Mode";
				new_tr.appendChild(new_td);
			}
			
			if (game_stats_hash['level'] != 0 && game_stats_hash['pvp'] && game_stats_hash['prof-pri'] && game_stats_hash['prof-sec'])
			{
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.colSpan = 2;
				new_td.innerText = "Level: " + game_stats_hash['level'];
				new_tr.appendChild(new_td);
  			
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.colSpan = 2;
				if (game_stats_hash['pvp'] == 1)
					new_td.innerText = "PvP-Only Character";			
				else
					new_td.innerText = "Roleplaying Character";
				new_tr.appendChild(new_td);
  			
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.colSpan = 2;
				new_td.innerText = game_stats_hash['prof-pri'] + "/" + game_stats_hash['prof-sec'];
				new_tr.appendChild(new_td);
			}
			
			if (game_stats_hash['name'])
			{
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_td = document.createElement("TD");
				new_td.colSpan = 2;
				new_td.className = "char_name";
				new_td.innerText = game_stats_hash['name'];
				new_tr.appendChild(new_td);
			}
		}
    }
}
