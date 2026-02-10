// DOM object should look as follows:

//<div id='pft_uploads_id' style='display: block'>
//	<div>" + "Uploads" + "</div>
//	<table>
//	<tbody id='pft_uploads_table_id'>
//		<tr id='uploads_row_1000'>
//			<td id='uploads_title_1000'>my_file.jpg</td>
//			<td id='uploads_percent_1000'>75%</td>
//			<td id='uploads_action_1000'><a href=' '>Cancel</a></td>
//		</tr>
//	</tbody>
//	</table>
//</div>

// NOTES:
// Your either a SENDER or a RECEIVER.  This will decide where the file transfer gets placed:
// Either under 'Uploads' or 'Downloads'

function GetPFTType(bSender)
{
	if (bSender == "1")
		return "uploads";
	else
		return "downloads";
}

function AddPFT(bSender, strFileId, strFileTitle, strPercentComplete, bCompleted)
{
	// Create the parent <div> id, either "pft_uploads_id" or "pft_downloads_id".
	var strType = GetPFTType(bSender);
	strParentDivId = "pft_" + strType + "_id";
	
	// Make sure the parent div object is visible
	var parent_element = document.getElementById(strParentDivId);
	parent_element.style.display = 'block';

	// See if we already have a row child object for this file id
	var strRowId = strType + "_row_" + strFileId;
	var row_element = document.getElementById(strRowId);
	if (!row_element)
	{
		// Get table element
		var strTableId = "pft_" + strType + "_table_id";
		var table_element = document.getElementById(strTableId);
		if (table_element)
		{
			// Create a new row element
			var newRow = document.createElement("tr");
			newRow.id = strRowId;

			// Column 1		
			var newCol = document.createElement("td");
			newCol.id = strType + "_title_" + strFileId;
			newCol.setAttribute("width", "50%");
			
			var newTextNode = document.createTextNode(strFileTitle);
			if (bCompleted == "1" && bSender == "0")
			{
				// If its completed I want to be able to click on the file title to open the files directory
				// <td class='buddy'><a href_type='PFT' file_id='1000' href=' '>my_file.jpg</a></td>
				var newHREF = document.createElement("a");
				newHREF.setAttribute("href_type", "PFT");
				newHREF.setAttribute("file_id", strFileId);
				newHREF.setAttribute("href", " ");
				newHREF.appendChild(newTextNode);
				newCol.appendChild(newHREF);
			}
			else
			{
				// <td class='buddy'>my_file.jpg</td>");
				newCol.appendChild(newTextNode);
			}
			newRow.appendChild(newCol);
				
			// Column 2
			newCol = document.createElement("td");
			newCol.id = strType + "_percent_" + strFileId;
			newCol.setAttribute("width", "25%");
			newTextNode = document.createTextNode(strPercentComplete);
			newCol.appendChild(newTextNode);
			newRow.appendChild(newCol);
			
			// Column 3
			newCol = document.createElement("td");
			newCol.id = strType + "_action_" + strFileId;
			newCol.setAttribute("width", "25%");
			if (bCompleted == "0")
			{
				// We only display 'Cancel' HREF if it's not completed yet.
				var newHREF = document.createElement("a");
				newHREF.setAttribute("href_type", "CancelPFT");
				newHREF.setAttribute("file_id", strFileId);
				newHREF.setAttribute("href", " ");
				newTextNode = document.createTextNode("Cancel");
				newHREF.appendChild(newTextNode);
				newCol.appendChild(newHREF);
			}
			newRow.appendChild(newCol);
			
			// Append row to table
			table_element.appendChild(newRow);
		}
	}
}

function UpdatePFT(bSender, strFileId, strPercentComplete)
{
	var strType = GetPFTType(bSender);
	
	// Find the correct rows <td> element to update the percent text
	var strTdId = strType + "_percent_" + strFileId;
	var td_element = document.getElementById(strTdId);
	if (td_element)
	{
		td_element.innerHTML = strPercentComplete;
	}	
}

function DeletePFT(bSender, strFileId)
{
	var strType = GetPFTType(bSender);
	
	// We better have a parent div element
	var strParentDivId = "pft_" + strType + "_id";
	var parent_element = document.getElementById(strParentDivId);
	if (parent_element)
	{
		// Get table element and remove all of its children
		var strTableId = "pft_" + strType + "_table_id";
		var table_element = document.getElementById(strTableId);
		if (table_element)
		{
			// Find the row we want to delete
			var strRowId = strType + "_row_" + strFileId;
			var row_element = document.getElementById(strRowId);
			if (row_element)
			{
				table_element.removeChild(row_element);
				row_element.removeNode(true);
				
				// Check the case that our table has no more rows, then hide section
				if (table_element.childNodes.length == 0)
				{
					parent_element.style.display = 'none';
				}
			}
		}
	}	
}

function CompletedPFT(bSender, strFileId, strTitle, strPercentComplete)
{
	var strType = GetPFTType(bSender);

	// Find the 'uploads_title_1000' td element
	var strTdId = strType + "_title_" + strFileId;
	var td_element = document.getElementById(strTdId);
	if (td_element)
	{
		// If we are the receiver of file, then I want to replace inner html with
		// an HREF element that they can click on to open directory.
		if (bSender == "0")
		{
			// Get rid of old text node
			var oldTextNode = td_element.firstChild;
			td_element.removeChild(oldTextNode);
			oldTextNode.removeNode(true);

			// Create new HREF element		
			var newHREF = document.createElement("a");
			newHREF.setAttribute("href_type", "PFT");
			newHREF.setAttribute("file_id", strFileId);
			newHREF.setAttribute("href", " ");
			
			// Add text node as child of this node
			var newTextNode = document.createTextNode(strTitle);
			newHREF.appendChild(newTextNode);
			
			// Append to parent (child_element)
			td_element.appendChild(newHREF);
		}
	}	

	// We have to update the percent text because the receiver doesn't get an UpdatePFT call when 100% done.
	UpdatePFT(bSender, strFileId, strPercentComplete);
	
	// Find the 'uploads_action_1000' td element
	strTdId = strType + "_action_" + strFileId;
	td_element = document.getElementById(strTdId);
	if (td_element)
	{
		// You're done, get rid of HREF node. (You can't 'Cancel' any more)
		var childNode = td_element.firstChild;
		td_element.removeChild(childNode);
		childNode.removeNode(true);
	}

}
