/////////////////////////////////
// Custom overrides for Quake4 //
/////////////////////////////////

function _(info, key, defaultval)
{
	if (info[key])
		return info[key];
	if (defaultval)
		return defaultval;
	return '';
}

function _int(info, key, defaultval)
{
	if (!defaultval)
		defaultval = '0';
	var ret = _(info, key, defaultval);
	return parseInt(ret);
}

// for urls to pages on the site
function _link(info, key)
{
	return _(info, 'siteurl') + _(info, key);
}

// for urls to images, and other media.
function _media(info, key)
{
	return _(info, 'mediaurl') + _(info, key);
}

function quake_live_create_info(info)
{
	// is this the correct format?
	/*
	info['players'] = [
      {
         "name": "Dr_Killer_UK",
         "score": "0",
         "model_url": "http://static.quakelive.com/images/players/icon_sm/bones_bones.jpg"
      },
      {
         "name": "ZaxAttack",
         "score": "0",
         "model_url": "http://static.quakelive.com/images/players/icon_sm/doom_default.jpg"
      },
      {
         "name": "Progun_",
         "score": "0",
		 "model_url": "http://static.quakelive.com/images/players/icon_sm/james_default.jpg"
      },
      {
         "name": "FrostyG",
         "score": "0",
         "model_url": "http://static.quakelive.com/images/players/icon_sm/uriel_default.jpg"
      }
   ];
   */
   
	var status = _(info,'status','Browsing Quake Live');
	var html = '';
	html += "<table id='quake_live_info' style='background-color:none' align='center'>";
	html += "<tr><td width='30'><img width='30' height='30' src='" + _media(info,'usericon') + "' style='border: 1px solid white;' /></td>"
	html += "<td valign='top' style='padding-left: 5px'><a href='" + _link(info,'profileurl') + "'>" + _(info,'username') + "</a><br>" + status + "</td></tr>";

	var inserver = _(info,'joinurl').length > 0;

	if (!inserver)
		html += "<tr><td colspan='2' align='center' style='padding-top: 15px'><a href='" + _link(info,'profileurl') + "'><h4>View Profile</h4></a></td></tr>";
	else
		html += "<tr><td colspan='2' align='center' style='padding-top: 15px'><a href='" + _link(info,'joinurl') + "'><h4>Join Game!</h4></a></td></tr>";
	html += "</table>";
	
	if (inserver)
	{
		var gametype = _(info,'gametypeshort');

		html += "<table id='quake_live_info' style='background-color:none' align='center' border='0'>";
		html += "<tr><td width='100' valign='top' align='center'>";
		html += "<div width='80' height='60' style='position: relative; overflow: visible'>";
		html += "<img width='80' height='60' src='" + _media(info, 'mapicon') + "' style='border: 1px solid white;' />";
		html += "<img width='28' height='28' src='" + _media(info, 'gametypeicon') + "' style='position: absolute; left: -4px; bottom: -4px' />";
		html += "</div>";
		html += "<br><b>" + _(info, 'maptitle') + "</b></td>";
		html += "<td valign='top'>";
		
		// TODO: Display only appropriate items
		var numclients = _int(info, 'numclients');
		html += "Players: " + numclients + "/" + _int(info, 'maxclients') + "<br>";
		if (gametype == "ffa" || gametype == "duel")
		{
			html += "Frag Limit: " + _int(info, 'fraglimit') + "<br>";
		}
		else if (gametype == "ctf")
		{
			html += "Red Score: " + _int(info, 'redscore') + "<br>";
			html += "Blue Score: " + _int(info, 'bluescore') + "<br>";
			html += "Capture Limit: " + _int(info, 'capturelimit') + "<br>";
		}
		else if (gametype == "ca")
		{
			html += "Red Score: " + _int(info, 'redscore') + "<br>";
			html += "Blue Score: " + _int(info, 'bluescore') + "<br>";
			html += "Round Limit: " + _int(info, 'roundlimit') + "<br>";
		}
		else if (gametype == "tdm")
		{
			html += "Red Score: " + _int(info, 'redscore') + "<br>";
			html += "Blue Score: " + _int(info, 'bluescore') + "<br>";
			html += "Frag Limit: " + _int(info, 'fraglimit') + "<br>";
			html += "Friendly Fire: " + ( _int(info, 'friendlyfire') ? 'On' : 'Off' ) + "<br>";
		}
	
		html += "Time Limit: " + _int(info, 'timelimit') + "<br>";
		html += "</td></tr>";
		html += "</table>";
		
		html += "<br>";
		
		if (numclients > 0)
		{
			html += "<table style='background-color:none' width='80%' align='center' border=1 cellspacing=0 cellpadding=0>";
			html += "    <tr>";
			html += "      <td height='20' style='line-height: 18px'>&nbsp;&nbsp;<b>Player</b></th>";
			html += "      <td width='50' style='line-height: 18px' align='center'><b>Score</b></th>";
			html += "    </tr>";

			for ( var i = 0; i < numclients; ++i )
			{
				if ( _(info,'p' + i + '_icon').length == 0 ) {
					break;	// end of list
				}
				var icon = _media(info, 'p' + i + '_icon');
				var name = _(info, 'p' + i + '_name');
				var clan = _(info, 'p' + i + '_clan');
				var score = _(info, 'p' + i + '_score');	// This CAN be a string in the case of spectators
				if ( clan.length > 0 )
				{
					name = clan + ' ' + name;
				}
				html += "<tr><td style='line-height: 18px; padding: 3px'><img style='margin-left: 0 5px; float: left' width='18' height='18' src='" + icon + "' /><div style='float: left; margin-left: 4px; line-height: 18px'>" + name + "</div><div style='clear: both'></div></td><td align='center' style='line-height: 18px'>" + score + "</td></tr>";
			}
		
			html += "</table>";
		}
	}
	return html;
}

var render_game_stats_hash = function()
{
	var game_stats_hash = { %game_stats_hash% };

	if (%has_game_stats_hash%)
	{
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
			
			var new_tr = tbody.insertRow(nInsertionRow);
			var new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			//new_th.innerText = game_stats_key;
			var new_td = document.createElement("TD");
			new_td.innerHTML = quake_live_create_info(game_stats_hash);
			new_td.colspan = 2;
			//new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);		

			//alert("insert at: " + nInsertionRow);
			/*
			/*
			for (var game_stats_key in game_stats_hash)
			{
				var new_tr = tbody.insertRow(nInsertionRow);
				var new_th = document.createElement("TH");
				new_th.className = "first_char_uppercase";
				new_th.innerText = game_stats_key;
				var new_td = document.createElement("TD");
				new_td.innerText = game_stats_hash[game_stats_key];
				new_tr.appendChild(new_th);
				new_tr.appendChild(new_td);
			}
			*/
		}
	}
}