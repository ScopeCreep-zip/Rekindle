		//////////////////////////////////////////////////////////////////////////////////////////////
		// CreateCheckBox()
		// - Helper function to create the following HTML DOM:
		// <div>
		//   <label for='official'><input id='official' type='checkbox' name='official'>Official</label>
		// </div>
		//////////////////////////////////////////////////////////////////////////////////////////////
		function CreateCheckBox(id, name, text)
		{
			var div_element = document.createElement("DIV");
			var label_element = document.createElement("LABEL");
			label_element.htmlFor = id;
			var input_element = document.createElement("INPUT");
			input_element.id = id;
			input_element.type = "checkbox";
			input_element.name = name;
			var text_element = document.createTextNode(text);
			
			label_element.appendChild(input_element);
			label_element.appendChild(text_element);
			div_element.appendChild(label_element);
			return div_element;
		}
		
		//////////////////////////////////////////////////////////////////////////////////////////////
		// CreateRow()
		// - Helper function to create the following HTML DOM:
		// <tr>
		//	 <th>%js:text_min_players%:</th>
		//	 <td><input id='xf_numplayers_min' class='text' type='text' size='15' name='xf_numplayers_min' onkeypress='disableEnterKey()'/></td>
		// </tr>
		//////////////////////////////////////////////////////////////////////////////////////////////
		function CreateRow(id, name, text)
		{
			var tr_element = document.createElement("TR");
			var th_element = document.createElement("TH");
			var text_element = document.createTextNode(text);
			var td_element = document.createElement("TD");
			var input_element = document.createElement("INPUT");
			input_element.id = id;
			input_element.className = "text";
			input_element.type = "text";
			input_element.size = 15;
			input_element.name = name;
			input_element.attachEvent("onkeypress", disableEnterKey);

			th_element.appendChild(text_element);
			td_element.appendChild(input_element);
			tr_element.appendChild(th_element);
			tr_element.appendChild(td_element);
			return tr_element;			
		}
		
		//////////////////////////////////////////////////////////////////////////////////////////////
		// RenderGameSpecificBox()
		// - Renders Americas Army Game Specific Filters.
		// - Allows game specific filters to render data in the custom_box.
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		RenderGameSpecificBox = function()
		{
			// Make sure the box is visible.
			var box = document.getElementById("game_specific_box");
			if (box)
				box.style.display = "block";
			
			// Checkboxes...
			var parent_element = document.getElementById("game_specific_checkboxes_here");
			if (parent_element)
			{
				var new_checkbox = CreateCheckBox("bf2_ranked", "bf2_ranked", "%js:text_ranked%");
				if (new_checkbox)
				{
					parent_element.appendChild(new_checkbox);
				}
					
				new_checkbox = CreateCheckBox("bf2_voip", "bf2_voip", "%js:text_voice%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);
				
				new_checkbox = CreateCheckBox("bf2_anticheat", "bf2_anticheat", "%js:text_punkbuster%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);

				new_checkbox = CreateCheckBox("bf2_friendlyfire", "bf2_friendlyfire", "%js:text_friendly_fire%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);

				new_checkbox = CreateCheckBox("bf2_pure", "bf2_pure", "%js:text_pure%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);

				new_checkbox = CreateCheckBox("bf2_globalunlocks", "bf2_globalunlocks", "%js:text_global_unlocks%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);

				new_checkbox = CreateCheckBox("bf2_autobalanced", "bf2_autobalanced", "%js:text_autobalanced%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);

				new_checkbox = CreateCheckBox("bf2_dedicated", "bf2_dedicated", "%js:text_dedicated%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);

				new_checkbox = CreateCheckBox("linux", "linux", "%js:text_only_linux_servers%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);

				new_checkbox = CreateCheckBox("windows", "windows", "%js:text_only_windows_servers%");
				if (new_checkbox)
					parent_element.appendChild(new_checkbox);

			}
			
			// Table rows...
			var parent_element = document.getElementById("game_specific_table_here");
			if (parent_element)
			{
				var new_row = CreateRow("timelimit_min", "timelimit_min", "%js:text_min_time_limit%");
				if (new_row)
					parent_element.appendChild(new_row);
					
				new_row = CreateRow("timelimit_max", "timelimit_max", "%js:text_max_time_limit%");
				if (new_row)
					parent_element.appendChild(new_row);

				new_row = CreateRow("roundtime_min", "roundtime_min", "%js:text_min_round_time%");
				if (new_row)
					parent_element.appendChild(new_row);

				new_row = CreateRow("roundtime_max", "roundtime_max", "%js:text_max_round_time%");
				if (new_row)
					parent_element.appendChild(new_row);

				new_row = CreateRow("bf2_ticketratio_min", "bf2_ticketratio_min", "%js:text_min_ticket_ratio%");
				if (new_row)
					parent_element.appendChild(new_row);

				new_row = CreateRow("bf2_ticketratio_max", "bf2_ticketratio_max", "%js:text_max_ticket_ratio%");
				if (new_row)
					parent_element.appendChild(new_row);

				new_row = CreateRow("bf2_teamratio_min", "bf2_teamratio_min", "%js:text_min_team_ratio%");
				if (new_row)
					parent_element.appendChild(new_row);
				
				new_row = CreateRow("bf2_teamratio_max", "bf2_teamratio_max", "%js:text_max_team_ratio%");
				if (new_row)
					parent_element.appendChild(new_row);
					
				new_row = CreateRow("bf2_mapsize_min", "bf2_mapsize_min", "%js:text_min_map_size%");
				if (new_row)
					parent_element.appendChild(new_row);
				
				new_row = CreateRow("bf2_mapsize_max", "bf2_mapsize_max", "%js:text_max_map_size%");
				if (new_row)
					parent_element.appendChild(new_row);
			}			
		}
		

		//////////////////////////////////////////////////////////////////////////////////////////////
		// GetFilters()
		// - Returns the string representation of the filter: i.e. xf_hideempty==1;protocol~~68;
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		GetFilters = function()
		{
			var xf_hideempty = document.getElementById('xf_hideempty');
			var xf_hidefull = document.getElementById('xf_hidefull');
			var xf_servername = document.getElementById('xf_servername');
			var xf_mapname = document.getElementById('xf_mapname');
			var xf_gametype = document.getElementById('xf_gametype');
			var xf_ping = document.getElementById('xf_ping');
			var xf_numplayers_min = document.getElementById('xf_numplayers_min');
			var xf_numplayers_max = document.getElementById('xf_numplayers_max');
			var xf_player = document.getElementById('xf_player');
			var country_combo = document.getElementById('xf_country');

			var bf2_voip = document.getElementById('bf2_voip');
			var bf2_ranked = document.getElementById('bf2_ranked');
			var bf2_pure = document.getElementById('bf2_pure');
			var bf2_globalunlocks = document.getElementById('bf2_globalunlocks');
			var bf2_friendlyfire = document.getElementById('bf2_friendlyfire');
			var bf2_dedicated = document.getElementById('bf2_dedicated');
			var bf2_autobalanced = document.getElementById('bf2_autobalanced');
			var bf2_anticheat = document.getElementById('bf2_anticheat');
			var linux = document.getElementById('linux');
			var windows = document.getElementById('windows');
			var timelimit_min = document.getElementById('timelimit_min');
			var timelimit_max = document.getElementById('timelimit_max');
			var roundtime_min = document.getElementById('roundtime_min');
			var roundtime_max = document.getElementById('roundtime_max');
			var bf2_ticketratio_min = document.getElementById('bf2_ticketratio_min');
			var bf2_ticketratio_max = document.getElementById('bf2_ticketratio_max');
			var bf2_teamratio_min = document.getElementById('bf2_teamratio_min');
			var bf2_teamratio_max = document.getElementById('bf2_teamratio_max');
			var bf2_mapsize_min = document.getElementById('bf2_mapsize_min');
			var bf2_mapsize_max = document.getElementById('bf2_mapsize_max');

			var str = "";

			if (xf_hideempty && xf_hideempty.checked)
			{
				str += "xf_hideempty==1;";
			}
			if (xf_hidefull && xf_hidefull.checked)
			{
				str += "xf_hidefull==1;";
			}
			if (bf2_voip && bf2_voip.checked)
			{
				str += "bf2_voip==1;";
			}
			if (bf2_ranked && bf2_ranked.checked)
			{
				str += "bf2_ranked==1;";
			}
			if (bf2_pure && bf2_pure.checked)
			{
				str += "bf2_pure==1;";
			}
			if (bf2_globalunlocks && bf2_globalunlocks.checked)
			{
				str += "bf2_globalunlocks==1;";
			}
			if (bf2_friendlyfire && bf2_friendlyfire.checked)
			{
				str += "bf2_friendlyfire==1;";
			}
			if (bf2_dedicated && bf2_dedicated.checked)
			{
				str += "bf2_dedicated==1;";
			}
			if (bf2_autobalanced && bf2_autobalanced.checked)
			{
				str += "bf2_autobalanced==1;";
			}
			if (bf2_anticheat && bf2_anticheat.checked)
			{
				str += "bf2_anticheat==1;";
			}
			if (linux && windows)
			{
				if (linux.checked && !windows.checked)
				{
					str += "bf2_os~~linux;";
				}
				if (windows.checked && !linux.checked)
				{
					str += "bf2_os~~win;";
				}
			}
			if (xf_servername && xf_servername.value != "")
			{
				str += "xf_servername~~" + escapeString(xf_servername.value) + ";";
			}
			if (xf_mapname && xf_mapname.value != "")
			{
				str += "xf_mapname~~" + escapeString(xf_mapname.value) + ";";
			}
			if (xf_gametype && xf_gametype.value != "")
			{
				str += "xf_gametype~~" + escapeString(xf_gametype.value) + ";";
			}
			if (xf_ping && xf_ping.value != "")
			{
				str += "xf_ping<=" + escapeString(xf_ping.value) + ";";
			}
			if (xf_numplayers_min && xf_numplayers_min.value != "")
			{
				str += "xf_numplayers>=" + escapeString(xf_numplayers_min.value) + ";";
			}
			if (xf_numplayers_max && xf_numplayers_max.value != "")
			{
				str += "xf_numplayers<=" + escapeString(xf_numplayers_max.value) + ";";
			}
			if (timelimit_min && timelimit_min.value != "")
			{
				str += "timelimit>=" + escapeString(timelimit_min.value) + ";";
			}
			if (timelimit_max && timelimit_max.value != "")
			{
				str += "timelimit<=" + escapeString(timelimit_max.value) + ";";
			}
			if (roundtime_min && roundtime_min.value != "")
			{
				str += "roundtime>=" + escapeString(roundtime_min.value) + ";";
			}
			if (roundtime_max && roundtime_max.value != "")
			{
				str += "roundtime<=" + escapeString(roundtime_max.value) + ";";
			}
			if (bf2_ticketratio_min && bf2_ticketratio_min.value != "")
			{
				str += "bf2_ticketratio>=" + escapeString(bf2_ticketratio_min.value) + ";";
			}
			if (bf2_ticketratio_max && bf2_ticketratio_max.value != "")
			{
				str += "bf2_ticketratio<=" + escapeString(bf2_ticketratio_max.value) + ";";
			}
			if (bf2_teamratio_min && bf2_teamratio_min.value != "")
			{
				str += "bf2_teamratio>=" + escapeString(bf2_teamratio_min.value) + ";";
			}
			if (bf2_teamratio_max && bf2_teamratio_max.value != "")
			{
				str += "bf2_teamratio<=" + escapeString(bf2_teamratio_max.value) + ";";
			}
			if (bf2_mapsize_min && bf2_mapsize_min.value != "")
			{
				str += "bf2_mapsize>=" + escapeString(bf2_mapsize_min.value) + ";";
			}
			if (bf2_mapsize_max && bf2_mapsize_max.value != "")
			{
				str += "bf2_mapsize<=" + escapeString(bf2_mapsize_max.value) + ";";
			}
			if (xf_player && xf_player.value != "")
			{
				str += "xf_player~~" + escapeString(xf_player.value) + ";";
			}
			if (country_combo)
			{
				var nSelectedIndex = country_combo.selectedIndex;
				var strVal = country_combo.options[nSelectedIndex].value;
				// Only save out if != "all"
				if (strVal != "all")
				{
					str += "xf_country~~" + strVal + ";";
				}
			}
			
			// Advanced filters
			///////////////////
			var table_element = document.getElementById("raw_table");
			if (table_element)
			{
				if (table_element.hasChildNodes() == true)
				{
					var node = table_element.firstChild;
					while (node)
					{
						if (node.nodeName == "TR")
						{
							// Each ROW should have 3 SELECT elements, one for KEY, one for EXPRESSION, one for VALUE.
							var select_elements = node.getElementsByTagName("SELECT");
							if (select_elements && select_elements.length == 3)
							{
								// key is select_element[0]
								var keySelect = select_elements[0];
								var strKey = keySelect.options[keySelect.selectedIndex].value;

								// expression is select_element[1]
								keySelect = select_elements[1];
								var strExpression = keySelect.options[keySelect.selectedIndex].value;
								
								// value is select_element[2]
								keySelect = select_elements[2];
								var strValue = keySelect.options[keySelect.selectedIndex].value;
								
								//alert("key: " + strKey + ", value: " + strValue);
								var strNone = "%js:text_combo_none%";
								if (strKey != strNone)
									str += strKey + strExpression + strValue + ";";
							}
						}
						node = node.nextSibling;
					}
				}
			}
					
			//alert("GetFilters: " + str);
			return str;
		}

		//////////////////////////////////////////////////////////////////////////////////////////////
		// ClearAll()
		// - Resets everything on the page.
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		ClearAll = function()
		{
			// combo box
			var element = document.getElementById('xf_country');
			if (element)
				element.selectedIndex = 0;
			
			// checkboxes
			element = document.getElementById('xf_hideempty');
			if (element)
				element.checked = false;
			element = document.getElementById('xf_hidefull');
			if (element)
				element.checked = false;
			element = document.getElementById('bf2_voip');
			if (element)
				element.checked = false;
			element = document.getElementById('bf2_ranked');
			if (element)
				element.checked = false;
			element = document.getElementById('bf2_pure');
			if (element)
				element.checked = false;
			element = document.getElementById('bf2_globalunlocks');
			if (element)
				element.checked = false;
			element = document.getElementById('bf2_friendlyfire');
			if (element)
				element.checked = false;
			element = document.getElementById('bf2_dedicated');
			if (element)
				element.checked = false;
			element = document.getElementById('bf2_autobalanced');
			if (element)
				element.checked = false;
			element = document.getElementById('bf2_anticheat');
			if (element)
				element.checked = false;
			element = document.getElementById('linux');
			if (element)
				element.checked = false;
			element = document.getElementById('windows');
			if (element)
				element.checked = false;
			
			// text entries
			element = document.getElementById('xf_servername');
			if (element)
				element.value = "";
			element = document.getElementById('xf_mapname');
			if (element)
				element.value = "";
			element = document.getElementById('xf_gametype');
			if (element)
				element.value = "";
			element = document.getElementById('xf_ping');
			if (element)
				element.value = "";
			element = document.getElementById('xf_numplayers_min');
			if (element)
				element.value = "";
			element = document.getElementById('xf_numplayers_max');
			if (element)
				element.value = "";
			element = document.getElementById('timelimit_min');
			if (element)
				element.value = "";
			element = document.getElementById('timelimit_max');
			if (element)
				element.value = "";
			element = document.getElementById('roundtime_min');
			if (element)
				element.value = "";
			element = document.getElementById('roundtime_max');
			if (element)
				element.value = "";
			element = document.getElementById('bf2_ticketratio_min');
			if (element)
				element.value = "";
			element = document.getElementById('bf2_ticketratio_max');
			if (element)
				element.value = "";
			element = document.getElementById('bf2_teamratio_min');
			if (element)
				element.value = "";
			element = document.getElementById('bf2_teamratio_max');
			if (element)
				element.value = "";
			element = document.getElementById('bf2_mapsize_min');
			if (element)
				element.value = "";
			element = document.getElementById('bf2_mapsize_max');
			if (element)
				element.value = "";
			element = document.getElementById('xf_player');
			if (element)
				element.value = "";
			
			// advanced filters
			var table_element = document.getElementById("raw_table");
			if (table_element)
			{
				// Remove all rows.
				while (table_element.rows.length > 0)
					table_element.deleteRow(0);
			}
			
			// If we don't have any raw server info then inform user to refresh the filter.
			if (associative_array_length(gFilterRawKeyValues) == 0)
			{
				var tr_element = document.createElement("TR");
				var th_element = document.createElement("TD");
				th_element.colSpan = 4;
				var text_element = document.createTextNode("%text_empty_rawserver_keyvalues%");
				th_element.appendChild(text_element);
				tr_element.appendChild(th_element);
				document.getElementById("raw_table").appendChild(tr_element);
			}
			else
			{
				// If we have server info key/values, then we will be wanting an ADD row button.
				// Show the one-and-only ADD row icon
				var tr_element = document.createElement("TR");
				var th_element = document.createElement("TH");
				var span_element = document.createElement("SPAN");
				span_element.id = "add_raw_row_id";
				span_element.className = "fake_href";
				span_element.setAttribute("name", "AddRemoveRow");
				span_element.attachEvent("onclick", OnAddRawRow);
				var img_element = document.createElement("IMG");
				img_element.src = "%media_template_folder%infoview/images/icon_add.gif";
				img_element.title = "%text_add%";
				span_element.appendChild(img_element);
				th_element.appendChild(span_element);
				tr_element.appendChild(th_element);
				tr_element.appendChild(document.createElement("TD"));
				tr_element.appendChild(document.createElement("TD"));
				tr_element.appendChild(document.createElement("TD"));
				document.getElementById("raw_table").appendChild(tr_element);
			}
			
		}
		
		//////////////////////////////////////////////////////////////////////////////////////////////
		// SetFilters()
		// - Called on PAGELOADDONE and whenever we want to reset the filter infoview.
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		SetFilters = function(filtersstr)
		{
			//alert("SetFilters: " + filtersstr);
			
			// First clear everything out.
			ClearAll();
			
			// Place filter data in appropriate fields.
			var bRawServerInfoAdded = false;
			var filters = splitEscaped(filtersstr);
			for (var i = 0; i < filters.length; i++)
			{
				var filter = parseFilter(filters[i]);
				if (filter != null)
				{
					var strKey = filter[0];
					var strExpression = filter[1];
					var strValue = filter[2];
					
					if (strKey == "bf2_os")
					{
						// Special handling for "bf2_os" element.
						if (strValue == "linux" )
						{
							obj = document.getElementById('linux');
						}
						else if (strValue == "win")
						{
							obj = document.getElementById('windows');
						}

						if (obj != null)
						{
							obj.checked = (strValue != 0);
						}
						continue;
					}
					
					var obj = null;
					if (strKey == "xf_numplayers" ||
						strKey == "timelimit" ||
						strKey == "roundtime" ||
						strKey == "bf2_ticketratio" ||
						strKey == "bf2_teamratio" ||
						strKey == "bf2_mapsize")
					{
						if (strExpression == "<=")
							obj = document.getElementById(strKey + "_max");
						else if (strExpression == ">=")
							obj = document.getElementById(strKey + "_min");
					}
					else
					{
						obj = document.getElementById(strKey);
					}
					
					if (obj)
					{
						// Must be an HTML element built into the filter template.
						if (obj.type == 'checkbox')
						{
							obj.checked = (strValue != 0);
						}
						else if (obj.type == 'text')
						{
							obj.value = strValue;
						}
						else
						{
							if (strKey == "xf_country")
							{
								for (var j = 0; j < obj.length; j++)
								{
									if (obj.options[j].value == strValue)
									{
										obj.options[j].selected = true;
									}
								}
							}
						}
					}
					else
					{
						// If it's not an HTML element in the filter template, then it must be an
						// advanced raw server key/value filter.  Add NEW items to raw server table.
						//alert("Add raw item: " + strKey + strExpression + strValue);
						AddRawKeyValue(strKey, strExpression, strValue);
						bRawServerInfoAdded = true;
					}
				}
			}

			// What the user sees underneath the Advanced Filters section depends on whether
			// the raw server data is empty and whether or not any raw key values were set.
			if (associative_array_length(gFilterRawKeyValues) != 0)
			{
				// We have raw server data but NO key values were selected, show combo box with <none> selected.
				if (bRawServerInfoAdded == false)
				{
					// Empty will default selection to <none>.
					AddRawKeyValue("", "", "");
				}
			}

			// Any time new elements are dynamically added/removed, we need to inform the client app.
			// Fire off an event which will tell the client to rebuild the html event sinks.
			RebuildEventSinks();
		}

