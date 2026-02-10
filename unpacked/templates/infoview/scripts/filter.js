		//////////////////////////////////////////////////////////////////////////////////////////////
		// Global raw server info key/values.
		// NOTE: This is an associative array.
		// { "key1", ["value1", "value2", "value3"], "key2", ["value1", "value2"] }
		//////////////////////////////////////////////////////////////////////////////////////////////
		var gFilterRawKeyValues = { %filter_raw_key_values% };

		//////////////////////////////////////////////////////////////////////////////////////////////
		// Return the length of an associative array
		//////////////////////////////////////////////////////////////////////////////////////////////
		function associative_array_length(associative_array)
		{
			var ret = 0;
			for (key in associative_array)
				ret++;
			return ret;
		}

		//////////////////////////////////////////////////////////////////////////////////////////////
		// Disable enter key
		//////////////////////////////////////////////////////////////////////////////////////////////
		function disableEnterKey() 
		{ 
		  if (window.event.keyCode == 13) window.event.keyCode = -1; 
		} 

		//////////////////////////////////////////////////////////////////////////////////////////////
		// RenderGameSpecificBox()
		// - Allows game specific filters to render data in the game_specific_box.
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		var RenderGameSpecificBox = function()
		{
			var element = document.getElementById("game_specific_box");
			if (element)
				element.style.display = "none";
		}
		
		//////////////////////////////////////////////////////////////////////////////////////////////
		// GetFilters()
		// - Returns the string representation of the filter: i.e. xf_hideempty==1;protocol~~68;
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		var GetFilters = function()
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
			
			var str = "";

			if (xf_hideempty && xf_hideempty.checked)
			{
				str += "xf_hideempty==1;";
			}
			if (xf_hidefull && xf_hidefull.checked)
			{
				str += "xf_hidefull==1;";
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
							// Each ROW should have 3 SELECT elements, one for KEY, one for expression, one for VALUE.
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
					
			return str;
		}

		//////////////////////////////////////////////////////////////////////////////////////////////
		// ClearAll()
		// - Resets everything on the page.
		// - NOTE:  This function is declared as a VARIABLE and can be overridden for custom filters.
		//////////////////////////////////////////////////////////////////////////////////////////////
		var ClearAll = function()
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
				var text_element = document.createTextNode("%js:text_empty_rawserver_keyvalues%");
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
				img_element.title = "%js:text_add%";
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
		var SetFilters = function(filtersstr)
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
					
					var obj = null;
					if (strKey == "xf_numplayers")
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
							if (strExpression == '==')
							{
								obj.checked = (strValue != 0);
							}
							else
							{
								obj.checked = (strValue == 0);
							}
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

		//////////////////////////////////////////////////////////////////////////////////////////////
		// GetLimitServersTo()
		// - Returns the value associated with limit servers to country.
		//////////////////////////////////////////////////////////////////////////////////////////////
		function GetLimitServersTo()
		{
			var combo = document.getElementById('xf_country');
			if (combo)
			{
				var nSelIndex = combo.selectedIndex;
				return combo.options[nSelIndex].value;
			}
			return "all";
		}

		//////////////////////////////////////////////////////////////////////////////////////
		// OnRawKeyChanged()
		// - A selection was made in the key combo box.
		// - Reloads the value combo box based on the selected key.
		//////////////////////////////////////////////////////////////////////////////////////
		function OnRawKeyChanged()
		{
			//alert("OnRawKeyChanged");
			
			// Who fired this event? (arguments[0] == event object)
			var select_element = arguments[0].srcElement;
			if (select_element)
			{
				// What was selected?
				var nSelIndex = select_element.selectedIndex;
				var strKey = select_element.options[nSelIndex].value;
				
				var select_values_element = select_element.parentNode.nextSibling.nextSibling.firstChild;
				if (select_values_element)
				{
					// Clear out OLD values
					while (select_values_element.length > 0)
					{
						select_values_element.remove(0);
					}
					
					// Add NEW values
					var strNone = "%js:text_combo_none%";
					if (strKey == strNone)
					{
						var newOption = new Option(strNone, strNone, false, false);
						select_values_element.add(newOption);
					}
					else
					{
						var vValues = gFilterRawKeyValues[strKey];
						for (var i = 0; i < vValues.length; i++)
						{
							var strValue = vValues[i];
							var newOption = new Option(strValue, strValue, false, false);
							select_values_element.add(newOption);
						}
					}
					
					// Any time new elements are dynamically added/removed, we need to inform the client app.
					// Fire off an event which will tell the client to rebuild the html event sinks.
					RebuildEventSinks();
				}
			}
			
		}

		//////////////////////////////////////////////////////////////////////////////////////
		// OnRawTDResized()
		// - Raw server info TD element is resizing based on window resize.  Dynamically
		//   resize the child combo box (select element).
		//////////////////////////////////////////////////////////////////////////////////////
		function OnRawTDResized()
		{
			// Who fired this event?
			var td_element = arguments[0].srcElement;
			if (td_element)
			{
				var td_coords = GetCoordinates(td_element);
				var select_element = td_element.firstChild;
				if (select_element)
				{
					select_element.style.width = td_coords.width - 5;
				}	
			}
		}
				
		//////////////////////////////////////////////////////////////////////////////////////
		// OnRemoveRawRow()
		// - Someone clicked on the 'Delete' row button.
		// - Removes the associated row.
		//////////////////////////////////////////////////////////////////////////////////////
		function OnRemoveRawRow()
		{
			// NOTE:  For some reason it is passing the IMG tag as the source of event
			// We need to figure out the row index so we can delete it from the table
			// <tbody><tr><th><span><img ...>
			var img_element = arguments[0].srcElement;
			if (img_element)
			{
				var tr_element = img_element.parentNode.parentNode.parentNode;
				//alert("tr_element.nodeName: " + tr_element.nodeName);
				if (tr_element && tr_element.nodeName == "TR")
				{
					var table_element = tr_element.parentNode;
					if (table_element)
					{
						table_element.deleteRow(tr_element.rowIndex);
						
						// Any time new elements are dynamically added/removed, we need to inform the client app.
						// Fire off an event which will tell the client to rebuild the html event sinks.
						RebuildEventSinks();
					}
				}
			}
		}
		
		//////////////////////////////////////////////////////////////////////////////////////
		// OnAddRawRow()
		// - Someone clicked on the 'Add' row button.
		// - Adds a new row (defaulted to <none>) second to last row.
		//////////////////////////////////////////////////////////////////////////////////////
		function OnAddRawRow()
		{
			// NOTE:  For some reason it is passing the IMG tag as the source of event
			//var source_element = arguments[0].srcElement;
			//alert("srcElement.nodeName: " + source_element.nodeName);
			
			// Add a default raw key/value, note that bAppend param is false.
			// Adds the new row above the 'Add' button.
			AddRawKeyValue("", "", "");

			// Any time new elements are dynamically added/removed, we need to inform the client app.
			// Fire off an event which will tell the client to rebuild the html event sinks.
			RebuildEventSinks();
		}
		
		//////////////////////////////////////////////////////////////////////////////////////
		// AddRawKeyValue()
		// - Given a key/value adds a new row to the 'raw_table'.
		// - Pass in empty key/value params to select <none> option.
		// - Appends row if bAppend param is true, else it will add second from last row.
		//////////////////////////////////////////////////////////////////////////////////////
		function AddRawKeyValue(strKey, strExpression, strValue)
		{
			//alert("AddRawKeyValue: " + strKey + ", " + strValue);
			
			var table_element = document.getElementById("raw_table");
			if (table_element)
			{
				// Create a new row element
				var newRow = document.createElement("TR");
				
				// Column 1 (ADD/REMOVE ICON)
				var newCol = document.createElement("TH");
				
				var newSpan = document.createElement("SPAN");
				newSpan.className = "fake_href";
				newSpan.setAttribute("name", "AddRemoveRow");
				newSpan.attachEvent("onclick", OnRemoveRawRow);
				
				var newImg = document.createElement("IMG");
				newImg.src = "%media_template_folder%infoview/images/icon_delete.gif";
				newImg.title = "%js:text_delete%";

				newSpan.appendChild(newImg);
				newCol.appendChild(newSpan);
				newRow.appendChild(newCol);
				
				// COLUMN 2 (KEY)
				// We select the passed in strKey value and then load the value combo based on this.
				////////////////////////////////////////////////////////////////////////////////////
				newCol = document.createElement("TD");
												
				var newKeyCombo = document.createElement("SELECT");
				newCol.attachEvent("onresize", OnRawTDResized);
				newKeyCombo.attachEvent("onchange", OnRawKeyChanged);

				// First item is always <none>.
				var strNone = "%js:text_combo_none%";
				var newOption = new Option(strNone, strNone, false, false);
				newKeyCombo.add(newOption);

				// Add all of the keys found in gFilterRawKeyValues.
				var bFoundString = false;
				var i = 1; // We've already added <none> option.
				var nFoundIndex = 0;
				for (key in gFilterRawKeyValues)
				{
					if (key == strKey)
					{
						bFoundString = true;
						nFoundIndex = i;
					}
					
					newOption = new Option(key, key, false, false);
					newKeyCombo.add(newOption);
					i++;
				}
				
				// Must not have any raw key/values (OR didn't find the key in list), manually add.
				if (bFoundString == false)
				{
					if (strKey.length == 0)
					{
						// Select <none> option.
						newKeyCombo.selectedIndex = 0;
					}
					else
					{
						newOption = new Option(strKey, strKey, false, false);
						newKeyCombo.add(newOption); // Append.
						newKeyCombo.selectedIndex = newKeyCombo.length-1; // Select it.
					}
				}
				else
				{
					newKeyCombo.selectedIndex = nFoundIndex;
				}
				
				newCol.appendChild(newKeyCombo);
				newRow.appendChild(newCol);

				// Column 3 (EXPRESSION)
				///////////////////
				newCol = document.createElement("TD");
				newCol.attachEvent("onresize", OnRawTDResized);
				var newValueCombo = document.createElement("SELECT");

				// First item is always "==".
				newOption = new Option("%js:text_starts_with%", "~~", false, false);
				newValueCombo.add(newOption);
				newOption = new Option("%js:text_equals%", "==", false, false);
				newValueCombo.add(newOption);
				newOption = new Option("%js:text_not_equals%", "!=", false, false);
				newValueCombo.add(newOption);
				newOption = new Option(">=", ">=", false, false);
				newValueCombo.add(newOption);
				newOption = new Option("<=", "<=", false, false);
				newValueCombo.add(newOption);

				nFoundIndex = 0;
                if (strExpression == "==")
                    nFoundIndex = 1;
                if (strExpression == "!=")
                    nFoundIndex = 2;
                else if (strExpression == ">=")
                    nFoundIndex = 3;
                else if (strExpression == "<=")
                    nFoundIndex = 4;
				newValueCombo.selectedIndex = nFoundIndex;
               
				newCol.appendChild(newValueCombo);
				newRow.appendChild(newCol);

				// Column 4 (VALUE)
				///////////////////
				newCol = document.createElement("TD");
				newCol.attachEvent("onresize", OnRawTDResized);
				var newValueCombo = document.createElement("SELECT");
				
				// First item is always <none>.
				newOption = new Option(strNone, strNone, false, false);
				newValueCombo.add(newOption);
				
				// Add all of the values for this key.
				bFoundString = false;
				nFoundIndex = 0;
				var valArray = gFilterRawKeyValues[strKey];
				if (valArray)
				{
					for (var j = 0; j < valArray.length; j++)
					{
						if (strValue == valArray[j])
						{
							bFoundString = true;
							nFoundIndex = j;
						}
						
						newOption = new Option(valArray[j], valArray[j], false, false);
						newValueCombo.add(newOption);
					}
				}
				
				// Must not have any raw key/values (OR didn't find the key in list), manually add.
				if (bFoundString == false)
				{
					if (strKey.length == 0)
					{
						// Select <none> option.
						newKeyCombo.selectedIndex = 0;
					}
					else
					{
						var newOption = new Option(strValue, strValue, false, false);
						newValueCombo.add(newOption); // Append
						newValueCombo.selectedIndex = newValueCombo.length-1; // Select
					}
				}
				else
				{
					newValueCombo.selectedIndex = nFoundIndex + 1; // +1 because of <none> option.
				}
				
				newCol.appendChild(newValueCombo);
				newRow.appendChild(newCol);

				// We only want to append the row if we do NOT have the ADD row button.
				// Otherwise add the new row second to last.
				if (document.getElementById("add_raw_row_id"))
				{
					// Second to last row
					if (table_element.rows.length >= 1)
						table_element.insertBefore(newRow, table_element.rows[table_element.rows.length-1]);
				}
				else
				{
					table_element.appendChild(newRow);
				}
			}
		}
				
		/* Overrides */
		%include filter.js%