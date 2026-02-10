%include scripts/AjaxRequest.js%

// custom overrides for world of warcraft

function getAbsoluteTop(obj)
{
	if (!obj)
		return 0;
	return obj.offsetTop + getAbsoluteTop(obj.offsetParent);
}

function getAbsoluteLeft(obj)
{
	if (!obj)
		return 0;
	return obj.offsetLeft + getAbsoluteLeft(obj.offsetParent);
}

function setupSlots()
{
	AjaxRequest.get({
		url:'%scripting_host%/client/games_stats/wow-item.xsl',
		onSuccess:function (req)
			{
				xsl = req.responseXML;
			  	if (document.getElementById('debug'))
				{
					document.getElementById('debug').value = "===\nXSL\n===\n";
					document.getElementById('debug').value += req.url + "\n";
			    		document.getElementById('debug').value += req.responseText;
				}
			},
		onError:function (req)
			{
				//alert("error getting xsl");
			}
	  });
	AjaxRequest.get({
		// url:'http://armory.worldofwarcraft.com/character-sheet.xml',
		url:'%scripting_host%/client/games_stats/wow-info.php',
		parameters:{username:'%username%'},
		onSuccess:function (req)
			{
				var chars = req.responseXML.getElementsByTagName('characters');
			  	if (document.getElementById('debugxsl'))
				{
					document.getElementById('debug').value = "===\nWoW Info XSL\n===\n";
					document.getElementById('debug').value += req.url + "\n";
			    		document.getElementById('debug').value += req.responseText;
				}
				var char_select = document.getElementById('char_selection');
				if (char_select && chars.length > 0)
				{
					var char = chars[0].firstChild;
					if (char)
					{
						var select_c = "<select id='charName' onchange='doWow()'>";
						while (char)
						{
							select_c += "<option value='";
							select_c += char.getAttribute('name').replace("'", '&#039;') + "-" + char.getAttribute('realm').replace("'", '&#039;') + "-" + char.getAttribute('region').replace("'", '&#039;');
							select_c += "'>" + char.getAttribute('name').replace("'", '&#039;') + "/" + char.getAttribute('realm').replace("'", '&#039;') + "</option>";
							
							char = char.nextSibling;
						}
						select_c += "</select>";
						char_select.innerHTML = "Character: " + select_c;

						var wow_box = document.getElementById('wow_box');
						if (wow_box)
						{
							wow_box.style.display = '';
							var wow = document.getElementById('wow_items');
							if (wow)
							{
								var t = getAbsoluteTop(wow);
								var l = getAbsoluteLeft(wow);
								for (var i = 0; i < 19; i++)
								{
									var div = document.getElementById('slot' + i);
									if (div)
									{
										if (i < 8)
										{
											div.style.top = (t + i*56) + "px";
											div.style.left = l + "px";
										}
										else if (i < 15)
										{
											div.style.top = (t + (i-8)*56) + "px";
											div.style.left = (l + 60 + 180) + "px";
										}
										else
										{
											div.style.top = (t + 7*56) + "px";
											div.style.left = (l + 60*(i-14)) + "px";
										}
										switch(i)
										{
											case 3:
												div.style.top = (t + 5*56) + "px";
												break;
											case 5:
												div.style.top = (t + 1*56) + "px";
												div.style.left = (l + 60 + 180) + "px";
												break;
											case 6:
												div.style.top = (t + 2*56) + "px";
												div.style.left = (l + 60 + 180) + "px";
												break;
											case 7:
												div.style.top = (t + 3*56) + "px";
												div.style.left = (l + 60 + 180) + "px";
												break;
											case 8:
												div.style.top = (t + 7*56) + "px";
												div.style.left = (l) + "px";
												break;
											case 9:
												div.style.top = (t + 0*56) + "px";
												div.style.left = (l + 60 + 180) + "px";
												break;
											case 10:
											case 11:
											case 12:
											case 13:
												div.style.top = (t + (i-6)*56) + "px";
												break;
											case 14:
												div.style.top = (t + 3*56) + "px";
												div.style.left = (l) + "px";
												break;
											case 18:
												div.style.top = (t + 6*56) + "px";
												div.style.left = (l) + "px";
												break;
										}
									}
								}
						
								var wow_info = document.getElementById('wow_info');
								if (wow_info)
								{
									wow_info.style.top = (t + 5) + "px";
									wow_info.style.left = (l + 65) + "px";
								}
							}
						}

						doWow();
						return;
					}
				}
			},
		onError:function (req)
			{
				//alert("error getting character list");
			}
	  });
}

var xsl = null;
var lang = "%language%";
var attempts = new Object();

function mouseOn(obj)
{
	if (!obj)
	{
		return;
	}

	var charName = "";
	var realmName = "";
	var region = "";

	var slct = document.getElementById('charName');
	if (slct)
	{
		var blob = slct.options[slct.selectedIndex].value;
		var idx = blob.indexOf('-');
		if (idx != -1)
		{
			charName = blob.substr(0, idx);
			idx++;
			var idx2 = blob.indexOf('-', idx);
			if (idx2 != -1)
			{
			  realmName = blob.substr(idx, idx2-idx);
			  region = blob.substr(idx2+1);
			}
		}
	}

	obj.style.backgroundPosition="-66px 0px";
	var wow_info = document.getElementById('wow_info');
	if (obj.getAttribute('itemid'))
	{
		AjaxRequest.get({
		        url:'%scripting_host%/client/games_stats/wow.php',
			// url:'http://armory.worldofwarcraft.com/item-tooltip.xml',
			parameters:{itemid:obj.getAttribute('itemid'), realm:realmName, char:charName, region:region, mode:'item'},
			onSuccess:function (req)
				{
				  	if (document.getElementById('debug'))
					{
						document.getElementById('debug').value = "===\nItem\n===\n";
						document.getElementById('debug').value += req.url;
				    		document.getElementById('debug').value += req.responseText;
					}

					var xfire = req.responseXML.getElementsByTagName('xfire');
					if (xfire.length != 0)
					{
						if(xfire[0].getAttribute('error'))
						{
							//alert("Error fetching XML: '" + xfire[0].getAttribute('str') + "'\ncode: " + xfire[0].getAttribute('error'));
							return;
						}
						//if (xfire[0].getAttribute('retry'))
						//{
						//	setTimeout("mouseOn(document.getElementById(" + obj.getAttribute('id') + "))", xfire[0].getAttribute('retry'));
						//}
					}

					var html = "";
					var page = req.responseXML.getElementsByTagName('page');
					if (page.length > 0)
					{
						if (lang == "de")
						{
							page[0].setAttribute('lang', 'de_de');
						}
						else if (lang == "es")
						{
							page[0].setAttribute('lang', 'es_es');
						}
						else if (lang == "fr")
						{
							page[0].setAttribute('lang', 'fr_fr');
						}
						if (xsl)
							html = req.responseXML.transformNode(xsl);
					}

					wow_info.innerHTML = html;
				},
			onError:function (req)
				{
					//alert('error getting item xml');
				}
			});
	}
	else
	{
		showCharInfo();
	}
}


function mouseOff(obj)
{
	obj.style.backgroundPosition="";
}

function showCharInfo()
{
	var wow_info = document.getElementById('wow_info');
	var char_tab = document.getElementById('char_tab');
	if (wow_info && char_tab)
	{
		wow_info.innerHTML = char_tab.innerHTML;
	}
}

function doWow()
{
	var charName = "";
	var realmName = "";
	var region = "";

	var slct = document.getElementById('charName');
	if (slct)
	{
		var blob = slct.options[slct.selectedIndex].value;
		var idx = blob.indexOf('-');
		if (idx != -1)
		{
			charName = blob.substr(0, idx);
			idx++;
			var idx2 = blob.indexOf('-', idx);
			if (idx2 != -1)
			{
			  realmName = blob.substr(idx, idx2-idx);
			  region = blob.substr(idx2+1);
			}
		}
	}

	if (charName != "" && realmName != "" && region != "")
	{
		if (attempts.last == (charName+'/'+realmName+'/'+region))
		{
			if (attempts.times > 3)
				return;	// too many attempts
			else
				attempts.times++;
		}
		else
		{
			attempts.last = charName+'/'+realmName+'/'+region;
			attempts.times = 0;
		}

		AjaxRequest.get({
		        // url:'http://armory.worldofwarcraft.com/character-sheet.xml',
			url:'%scripting_host%/wowchar/',
			parameters:{realm:realmName, name:charName, region:region},
			onSuccess:function (req)
			          {
				  	if (document.getElementById('debug'))
					{
						document.getElementById('debug').value = "===\nCharacter\n===\n";
						document.getElementById('debug').value += req.url + "\n";
				    		document.getElementById('debug').value += req.responseText;
					}

					if(req.responseText.substr(0,9) == "while(1);")
						req.responseText = req.responseText.substring(9);

					var data = "";
					eval( 'data=' + req.responseText );
					if (typeof(data) != 'object' || !data.name)
						return;
					if (data.retry)
						setTimeout("doWow()", data.retry);
					//document.getElementById('debug').value = dump(data);

					var wow = document.getElementById('wow_items');
					var char_box = document.getElementById('char_box');
					if (char_box)
					{
						char_box.style.visibility = "";
						var char_img = document.getElementById('char_img');
						if (char_img)
							char_img.src = data.region_base + data.portrait_path;
							
						var char_name_anchor = document.getElementById('char_name_anchor');
						if (char_name_anchor)
							char_name_anchor.innerHTML = data.name;

						if (data.guild && data.guild.length > 0)
						{
							var guild_name = document.getElementById('guild_name');
							if (guild_name)
								guild_name.innerHTML = "&lt;" + data.guild + "&gt;";
						}

						var char_info = document.getElementById('char_info');
						if (char_info)
							char_info.innerHTML = data.strs.level + " " + data.level + " " + data.strs.race + " " + data.strs['class'];
					}
	
					var not_found = document.getElementById('char_not_found');
					if (data.items.length == 0)
					{
						if (not_found)
							not_found.style.display = "";
						return;
					}
						
					if (not_found)
						not_found.style.display = "none";
	
					for (var i = 0; i < 19; i++)
					{
						var frame = document.getElementById('frame'+i);
						if (frame)
						{
							frame.removeAttribute('itemid');
						}
						var div = document.getElementById('slot'+i);
						if (div)
						{
							div.style.backgroundImage = '';
						}
					}
					var wow_info = document.getElementById('wow_info');
					if (wow_info)
					{
						wow_info.innerHTML = '';
						var char_info = document.getElementById('char_tab');
						setString('talentStr', data.strs.talent_spec_str);
						setString('healthStr', data.strs.health);
						var talentLink = document.getElementById('talentLink');
						if (talentLink)
							talentLink.href = data.region_base + '/character-talents.xml?' + data.char_url;

						setString('talentSpecStr', data.strs.talent_spec);
						document.getElementById('talentSpecImage').src = data.region_base + data.talent_spec_path;

						setString('talentVals1', data.talent_spec[0]);
						setString('talentVals2', data.talent_spec[1]);
						setString('talentVals3', data.talent_spec[2]);
						setString('secondBarStr', data.strs.second_bar);
						document.getElementById('secondBar').style.backgroundImage = 'url(' + data.region_base + data.second_bar_path + ')';
						document.getElementById('healthBar').style.backgroundImage = 'url(' + data.region_base + '/_images/bar-life.gif)';
						setString('healthBar', data.health_bar);
						setString('secondBar', data.second_bar);

						wow_info.innerHTML = char_info.innerHTML;
					}
					for (slot in data.items)
					{
						var div = document.getElementById('slot' + slot);
						if (div)
						{
							div.style.backgroundImage = 'url(http://www.wowarmory.com/wow-icons/_images/51x51/' + data.items[slot].icon + '.jpg)';
							var frame = document.getElementById('frame'+slot);
							frame.setAttribute('itemid', data.items[slot].id);
						}
					}
					RebuildEventSinks();
				  },
			onError:function (req)
			        {
					//alert("error for '" + req.url + "': " + req.statusText + "(" + req.status + ")");
				}
		});
	}
}

function setString(docid, str)
{
	var elem = document.getElementById(docid);
	if (elem)
		elem.innerHTML = str;
}

function fillInXPath(doc_id, xml_dom, node_id)
{
	var elem = document.getElementById(doc_id);
	if (elem && xml_dom)
	{
		var xml_elem = xml_dom.selectSingleNode(node_id);
		if (xml_elem)
		{
			elem.innerHTML = xml_elem.text;
		}
	}
}

function sortArray(a,b)
{
	return b[1] - a[1];
}

render_game_stats_hash = function()
{
	var game_stats_hash = { %game_stats_hash% };

	if (%has_game_stats_hash% && game_stats_hash['realmName'])
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

			//alert("insert at: " + nInsertionRow);
			var new_tr = tbody.insertRow(nInsertionRow);
			var new_th = document.createElement("TH");
			new_th.className = "first_char_uppercase";
			new_th.innerText = "Realm";
			var new_td = document.createElement("TD");
			new_td.innerText = game_stats_hash['realmName'];
			new_tr.appendChild(new_th);
			new_tr.appendChild(new_td);
		}
	}
}
function dump(arr,level) {
	var dumped_text = "";
	if(!level) level = 0;
	
	//The padding given at the beginning of the line.
	var level_padding = "";
	for(var j=0;j<level+1;j++) level_padding += "    ";
	
	if(typeof(arr) == 'object') { //Array/Hashes/Objects 
		for(var item in arr) {
			var value = arr[item];
			
			if(typeof(value) == 'object') { //If it is an array,
				dumped_text += level_padding + "'" + item + "' ...\n";
				dumped_text += dump(value,level+1);
			} else {
				dumped_text += level_padding + "'" + item + "' => \"" + value + "\"\n";
			}
		}
	} else { //Stings/Chars/Numbers etc.
		dumped_text = "===>"+arr+"<===("+typeof(arr)+")";
	}
	return dumped_text;
}
