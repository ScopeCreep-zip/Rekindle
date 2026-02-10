/////////////////////////////////
// Custom overrides for Mame32 //
/////////////////////////////////

// Include AJAX library.
%include scripts\AjaxRequest.js%

/////////////////////////////////////////////////////////////////////////////////////////////
// render_mame_game_info()
// - Response XML contains the 'bio' for the current MAME game.
/////////////////////////////////////////////////////////////////////////////////////////////
function render_mame_game_info(response_xml)
{
	//alert("xml = " + response_xml.xml);
	
	var extra1 = document.getElementById("extraDiv1");
	if (extra1)
	{
		// Clear out any old data (i.e. error messages) that may be inside of extra1 tag.
		while (extra1.childNodes.length)
		{
			extra1.removeChild(extra1.firstChild);
		}

		// Insert the game biography into extra1 tag.	
		var bio = response_xml.getElementsByTagName("bio");
		if (bio && bio[0])
		{
			var new_table = document.createElement("TABLE");
			var new_tr = new_table.insertRow(-1);
			var new_td = new_tr.insertCell(-1);
			new_td.className = "game_bio_title";
			new_td.innerText = "Game Bio";
			new_tr = new_table.insertRow(-1);
			new_td = new_tr.insertCell(-1);

			// Regular expression to replace all CR with <br> so I don't have to use a dumb PRE tag.			
			var strReplacedCR = bio[0].text.replace(/\n/g, "<br>");
			new_td.innerHTML = strReplacedCR;

			// Give credit where credit is due...
			new_tr = new_table.insertRow(-1);
			new_td = new_tr.insertCell(-1);
			new_td.innerHTML = "Mame History Provided by Alexis Bousiges<br><a href='http://www.arcade-history.com' target='_blank'>www.arcade-history.com</a>";

			// Append the table and make sure style is set to display.
			extra1.appendChild(new_table);
			extra1.style.display = "block";
			
			// Any time new elements are dynamically added/removed, we need to inform the client app.
			// Fire off an event which will tell the client to rebuild the html event sinks.
			RebuildEventSinks();
		}	
	}
}

function render_error(error_msg)
{
	var extra1 = document.getElementById("extraDiv1");
	if (extra1)
	{
		extra1.innerHTML = "<span>" + error_msg + "  <span class='fakelink' onClick='request_mame_data();'>Retry</span></span>";
		extra1.style.display = "block";
	}
	
}

/////////////////////////////////////////////////////////////////////////////////////////////
// request_mame_data()
// - Send off an AJAX request to server to get extra information about this MAME game.
/////////////////////////////////////////////////////////////////////////////////////////////
function request_mame_data()
{
	var game_stats_hash = { %game_stats_hash% };
	var game_short_name = game_stats_hash['short_name'];
	if (!game_short_name || !game_short_name.length)
		return;
	
	// AJAX the server for some data about this game
	AjaxRequest.get(
		{
			'url':'%scripting_host%/v4/client/mame32.php',
			'parameters': { 'short_name': game_short_name },
			'timeout':10000,
			'onSuccess':
				function (req)
				{
					if (req.responseXML)
					{
						render_mame_game_info(req.responseXML);
					}
				},
			'onError':
				function (req)
				{
					render_error("There was an error connecting to the server, please try again in a few minutes.");
				},
			'onTimeout':
				function (req)
				{
					render_error("The server took too long to respond, please try again in a few minutes.");
				}
		}
	);
}

/////////////////////////////////////////////////////////////////////////////////////////////
// render_server_extra()
// - Send off an AJAX request to server to get extra information about this MAME game.
/////////////////////////////////////////////////////////////////////////////////////////////
render_server_extra = function()
{
	//alert("Mame32 render_server_extra()");
	
	if (!%has_game_stats_hash%)
		return;
	
	request_mame_data();
}

/////////////////////////////////////////////////////////////////////////////////////////////
// render_game_stats_hash()
// - Override function because there are key/values in here that I don't want to display.
// - I really only want to display the name of the MAME game.
/////////////////////////////////////////////////////////////////////////////////////////////
render_game_stats_hash = function()
{
	if (!%has_game_stats_hash%)
		return;
		
	var tbody = document.getElementById("server_tbody_id");
	if (tbody)
	{
		var game_stats_hash = { %game_stats_hash% };
		
		// Look for 'Game' key/value
		if (game_stats_hash['Game'])
		{
			var new_tr = tbody.insertRow(-1); // append
			var new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "Game";
			var new_td = document.createElement("TD");
			new_td.innerText = game_stats_hash['Game'];
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}
	}
}
